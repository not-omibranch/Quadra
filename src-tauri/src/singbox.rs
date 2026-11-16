//! Turns a server plus the settings into the config sing-box is started with.
//!
//! Servers are stored the way Xray describes them (an outbound built from a share link, or a
//! provider's whole Xray config). This module translates that into sing-box at connect time,
//! so nothing stored on disk changes. Whatever sing-box (with our XHTTP transport) cannot
//! express is reported by [`needs_xray`]; such a server is still run by Xray.

use crate::model::{Server, Settings};
use crate::sys;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const TUN_TAG: &str = "tun";
/// Tag of the urltest group behind "Авто, самый быстрый".
pub const AUTO_TAG: &str = "auto";
/// Tag of the inbound the connection check uses.
pub const PROBE_TAG: &str = "probe";
const PROBE_URL: &str = "https://www.gstatic.com/generate_204";

/// Rule-sets that ship with the app (resources/sing-box/rules/<name>.srs).
pub const SHIPPED_RULE_SETS: &[&str] = &["geosite-category-ru", "geoip-ru"];

const PRIVATE_NETWORKS: &[&str] = &[
    "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "127.0.0.0/8", "169.254.0.0/16", "100.64.0.0/10",
    "fc00::/7", "fe80::/10", "::1/128",
];

/// Where the core's local side lives for one connection.
pub struct Local<'a> {
    pub tun: bool,
    pub clash_port: u16,
    /// A SOCKS port only for the connection check (0: none). Its traffic goes straight to the
    /// server, past DNS, routing presets and exclusions, so the check asks "does the server
    /// carry traffic" and nothing else. Before 0.4 the check went through the user's port and,
    /// with the Russia preset, first resolved its name with UDP DNS through the server: on a
    /// lossy path that alone stalled it for seconds and failed servers that worked.
    pub probe_port: u16,
    pub secret: &'a str,
    /// Folder with the shipped `.srs` rule-sets.
    pub rules_dir: &'a Path,
}

/// A config plus what was left out of it on the way (shown in the log).
pub struct Built {
    pub config: Value,
    pub notes: Vec<String>,
    /// For the "auto" group: outbound tag -> server id.
    pub members: HashMap<String, String>,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn listen(st: &Settings) -> &'static str {
    if st.allow_lan { "0.0.0.0" } else { "127.0.0.1" }
}

/// Xray's ranges come as a number or as "a-b"; ours are always strings.
fn range(v: &Value) -> Option<Value> {
    match v {
        Value::Number(n) => Some(json!(n.to_string())),
        Value::String(t) if !t.trim().is_empty() => Some(json!(t.trim())),
        Value::Object(o) => {
            let from = o.get("from").and_then(Value::as_i64)?;
            let to = o.get("to").and_then(Value::as_i64).unwrap_or(from);
            Some(json!(if from == to { from.to_string() } else { format!("{from}-{to}") }))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- outbounds

fn utls(fp: &str) -> Option<&'static str> {
    Some(match fp.to_ascii_lowercase().as_str() {
        "chrome" => "chrome",
        "firefox" => "firefox",
        "safari" => "safari",
        "ios" => "ios",
        "android" => "android",
        "edge" => "edge",
        "360" => "360",
        "qq" => "qq",
        "random" => "random",
        "randomized" | "randomizednoalpn" => "randomized",
        _ => return None,
    })
}

/// streamSettings.security → sing-box `tls` object (None for plain).
fn tls(stream: &Value, default_alpn: Option<&[&str]>) -> Result<Option<Value>, String> {
    match s(&stream["security"]) {
        "" | "none" => Ok(None),
        "tls" => {
            let t = &stream["tlsSettings"];
            let mut o = Map::new();
            o.insert("enabled".into(), json!(true));
            if !s(&t["serverName"]).is_empty() {
                o.insert("server_name".into(), json!(s(&t["serverName"])));
            }
            if t["allowInsecure"].as_bool() == Some(true) {
                let pinned = ["pinnedPeerCertificateChainSha256", "pinnedPeerCertSha256", "pinnedPeerCertificatePublicKeySha256"]
                    .iter().any(|k| !t[*k].is_null() && t[*k] != json!([]) && t[*k] != json!(""));
                if pinned {
                    return Err("TLS с закреплённым сертификатом".into());
                }
                o.insert("insecure".into(), json!(true));
            }
            let alpn: Vec<String> = match &t["alpn"] {
                Value::Array(a) => a.iter().filter_map(|v| v.as_str()).filter(|v| !v.is_empty()).map(String::from).collect(),
                Value::String(a) => a.split(',').map(str::trim).filter(|v| !v.is_empty()).map(String::from).collect(),
                _ => vec![],
            };
            if alpn.iter().any(|a| a == "h3") && stream_network(stream) == "xhttp" {
                return Err("XHTTP поверх HTTP/3".into());
            }
            if !alpn.is_empty() {
                o.insert("alpn".into(), json!(alpn));
            } else if let Some(d) = default_alpn {
                o.insert("alpn".into(), json!(d));
            }
            if let Some(fp) = utls(s(&t["fingerprint"])) {
                o.insert("utls".into(), json!({"enabled": true, "fingerprint": fp}));
            }
            Ok(Some(Value::Object(o)))
        }
        "reality" => {
            let r = &stream["realitySettings"];
            if !s(&r["mldsa65Verify"]).is_empty() {
                return Err("REALITY с проверкой ML-DSA-65".into());
            }
            if s(&r["publicKey"]).is_empty() {
                return Err("у REALITY нет открытого ключа".into());
            }
            let fp = utls(s(&r["fingerprint"])).unwrap_or("chrome");
            let mut o = json!({"enabled": true,
                "utls": {"enabled": true, "fingerprint": fp},
                "reality": {"enabled": true, "public_key": s(&r["publicKey"]), "short_id": s(&r["shortId"])}});
            if !s(&r["serverName"]).is_empty() {
                o["server_name"] = json!(s(&r["serverName"]));
            }
            Ok(Some(o))
        }
        other => Err(format!("защита {other}")),
    }
}

fn stream_network(stream: &Value) -> &str {
    match s(&stream["network"]) {
        "" | "raw" | "tcp" => "tcp",
        "splithttp" | "h2" | "http" => "xhttp",
        other => other,
    }
}

fn headers(v: &Value) -> Map<String, Value> {
    v.as_object().map(|o| o.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("host")).map(|(k, v)| (k.clone(), json!(s(v)))).collect())
        .unwrap_or_default()
}

/// XHTTP settings in our fork's names. `extra` replaces everything but host, path and mode,
/// as in Xray.
fn xhttp(x: &Value) -> Result<Value, String> {
    let base = if x["extra"].is_object() { &x["extra"] } else { x };
    if !base["downloadSettings"].is_null() {
        return Err("XHTTP с отдельным сервером для загрузки (downloadSettings)".into());
    }
    let mut o = Map::new();
    o.insert("type".into(), json!("xhttp"));
    if !s(&x["host"]).is_empty() {
        o.insert("host".into(), json!(s(&x["host"])));
    }
    if !s(&x["path"]).is_empty() {
        o.insert("path".into(), json!(s(&x["path"])));
    }
    let mode = match s(&x["mode"]) { "" => "auto", m => m };
    if !matches!(mode, "auto" | "packet-up" | "stream-up" | "stream-one") {
        return Err(format!("режим XHTTP {mode}"));
    }
    o.insert("mode".into(), json!(mode));
    let h = headers(&base["headers"]);
    if !h.is_empty() {
        o.insert("headers".into(), Value::Object(h));
    }
    for (from, to) in [("xPaddingBytes", "x_padding_bytes"), ("scMaxEachPostBytes", "sc_max_each_post_bytes"),
        ("scMinPostsIntervalMs", "sc_min_posts_interval_ms"), ("scStreamUpServerSecs", "sc_stream_up_server_secs"),
        ("uplinkChunkSize", "uplink_chunk_size")] {
        if let Some(r) = range(&base[from]) {
            o.insert(to.into(), r);
        }
    }
    for (from, to) in [("noGRPCHeader", "no_grpc_header"), ("noSSEHeader", "no_sse_header"), ("xPaddingObfsMode", "x_padding_obfs_mode")] {
        if base[from].as_bool() == Some(true) {
            o.insert(to.into(), json!(true));
        }
    }
    if let Some(n) = base["scMaxBufferedPosts"].as_i64().filter(|n| *n > 0) {
        o.insert("sc_max_buffered_posts".into(), json!(n));
    }
    for (from, to) in [("uplinkHTTPMethod", "uplink_http_method"), ("xPaddingKey", "x_padding_key"), ("xPaddingHeader", "x_padding_header"),
        ("xPaddingPlacement", "x_padding_placement"), ("xPaddingMethod", "x_padding_method"), ("sessionIDPlacement", "session_id_placement"),
        ("sessionIDKey", "session_id_key"), ("seqPlacement", "seq_placement"), ("seqKey", "seq_key"),
        ("uplinkDataPlacement", "uplink_data_placement"), ("uplinkDataKey", "uplink_data_key")] {
        if !s(&base[from]).is_empty() {
            o.insert(to.into(), json!(s(&base[from])));
        }
    }
    if let Some(m) = base["xmux"].as_object() {
        let mut xm = Map::new();
        for (from, to) in [("maxConcurrency", "max_concurrency"), ("maxConnections", "max_connections"), ("cMaxReuseTimes", "c_max_reuse_times"),
            ("hMaxRequestTimes", "h_max_request_times"), ("hMaxReusableSecs", "h_max_reusable_secs")] {
            if let Some(r) = m.get(from).and_then(range) {
                xm.insert(to.into(), r);
            }
        }
        if let Some(k) = m.get("hKeepAlivePeriod").and_then(Value::as_i64).filter(|k| *k != 0) {
            xm.insert("h_keep_alive_period".into(), json!(k));
        }
        if !xm.is_empty() {
            o.insert("xmux".into(), Value::Object(xm));
        }
    }
    Ok(Value::Object(o))
}

/// streamSettings.network → sing-box `transport` (None for raw TCP).
fn transport(stream: &Value) -> Result<Option<Value>, String> {
    Ok(match stream_network(stream) {
        "tcp" => {
            let header = &stream["tcpSettings"]["header"]["type"];
            if !header.is_null() && s(header) != "none" {
                return Err("TCP с маскировкой под HTTP".into());
            }
            None
        }
        "ws" => {
            let w = &stream["wsSettings"];
            let mut o = json!({"type": "ws"});
            let mut path = s(&w["path"]).to_string();
            // "?ed=2048": early data in the Sec-WebSocket-Protocol header, as Xray does it
            if let Some((p, q)) = path.clone().split_once('?') {
                let ed = q.split('&').find_map(|kv| kv.strip_prefix("ed=")).and_then(|v| v.parse::<u32>().ok());
                if let Some(ed) = ed {
                    path = p.to_string();
                    o["max_early_data"] = json!(ed);
                    o["early_data_header_name"] = json!("Sec-WebSocket-Protocol");
                }
            }
            if !path.is_empty() {
                o["path"] = json!(path);
            }
            let mut h = headers(&w["headers"]);
            let host = if s(&w["host"]).is_empty() { s(&w["headers"]["Host"]) } else { s(&w["host"]) };
            if !host.is_empty() {
                h.insert("Host".into(), json!(host));
            }
            if !h.is_empty() {
                o["headers"] = Value::Object(h);
            }
            Some(o)
        }
        "grpc" => {
            let g = &stream["grpcSettings"];
            let mut o = json!({"type": "grpc"});
            if !s(&g["serviceName"]).is_empty() {
                o["service_name"] = json!(s(&g["serviceName"]));
            }
            Some(o)
        }
        "httpupgrade" => {
            let h = &stream["httpupgradeSettings"];
            let mut o = json!({"type": "httpupgrade"});
            if !s(&h["host"]).is_empty() {
                o["host"] = json!(s(&h["host"]));
            }
            if !s(&h["path"]).is_empty() {
                o["path"] = json!(s(&h["path"]));
            }
            let hd = headers(&h["headers"]);
            if !hd.is_empty() {
                o["headers"] = Value::Object(hd);
            }
            Some(o)
        }
        "xhttp" => {
            let x = if stream["xhttpSettings"].is_object() { &stream["xhttpSettings"] } else if stream["splithttpSettings"].is_object() {
                &stream["splithttpSettings"]
            } else {
                &stream["httpSettings"]
            };
            Some(xhttp(x)?)
        }
        "hysteria" => None,
        other => return Err(format!("транспорт {other}")),
    })
}

/// Server address and port of an Xray outbound (vnext / servers / settings itself).
fn endpoint(ob: &Value) -> (&Value, String, u16) {
    let st = &ob["settings"];
    let first = if st["vnext"].is_array() { &st["vnext"][0] } else if st["servers"].is_array() { &st["servers"][0] } else { st };
    (first, s(&first["address"]).to_string(), first["port"].as_u64().unwrap_or(0) as u16)
}

/// One Xray proxy outbound → one sing-box outbound.
pub fn outbound(ob: &Value, tag: &str) -> Result<Value, String> {
    let proto = s(&ob["protocol"]);
    if ob["mux"]["enabled"].as_bool() == Some(true) {
        return Err("мультиплексирование Xray (mux)".into());
    }
    let stream = &ob["streamSettings"];
    let (first, server, port) = endpoint(ob);
    if server.is_empty() || port == 0 {
        return Err(format!("у исходящего {tag} нет адреса сервера"));
    }
    let net = stream_network(stream);
    let default_alpn: Option<&[&str]> = if net == "xhttp" { Some(&["h2", "http/1.1"]) } else { None };
    let mut o = json!({"tag": tag, "server": server, "server_port": port});
    match proto {
        "vless" => {
            let u = &first["users"][0];
            let enc = s(&u["encryption"]);
            if !enc.is_empty() && enc != "none" {
                return Err("шифрование VLESS".into());
            }
            o["type"] = json!("vless");
            o["uuid"] = json!(s(&u["id"]));
            match s(&u["flow"]) {
                "" | "none" => {}
                f if f.starts_with("xtls-rprx-vision") => o["flow"] = json!("xtls-rprx-vision"),
                f => return Err(format!("flow {f}")),
            }
        }
        "vmess" => {
            let u = &first["users"][0];
            o["type"] = json!("vmess");
            o["uuid"] = json!(s(&u["id"]));
            o["security"] = json!(match s(&u["security"]) { "" => "auto", x => x });
            if let Some(a) = u["alterId"].as_u64().filter(|a| *a > 0) {
                o["alter_id"] = json!(a);
            }
        }
        "trojan" => {
            o["type"] = json!("trojan");
            o["password"] = json!(s(&first["password"]));
        }
        "shadowsocks" => {
            o["type"] = json!("shadowsocks");
            o["method"] = json!(s(&first["method"]));
            o["password"] = json!(s(&first["password"]));
            if net != "tcp" {
                return Err("Shadowsocks с транспортом".into());
            }
        }
        "hysteria" => {
            if ob["settings"]["version"].as_i64().unwrap_or(2) != 2 || net != "hysteria" {
                return Err("Hysteria версии 1".into());
            }
            if !stream["finalmask"].is_null() || !stream["udpmasks"].is_null() {
                return Err("Hysteria с маскировкой UDP".into());
            }
            o["type"] = json!("hysteria2");
            o["password"] = json!(s(&stream["hysteriaSettings"]["auth"]));
            let mut t = tls(stream, None)?.ok_or("Hysteria без TLS")?;
            t["alpn"] = json!(["h3"]);
            if let Some(m) = t.as_object_mut() {
                m.remove("utls");
            }
            o["tls"] = t;
            return Ok(o);
        }
        other => return Err(format!("протокол {other}")),
    }
    if proto != "shadowsocks" {
        if let Some(t) = tls(stream, default_alpn)? {
            o["tls"] = t;
        }
    } else if !matches!(s(&stream["security"]), "" | "none") {
        return Err("Shadowsocks с TLS".into());
    }
    if let Some(t) = transport(stream)? {
        // Vision rides on raw TCP only (Xray refuses it over ws/grpc too); over XHTTP Xray
        // lets it through, and so do we
        if o["flow"].is_string() && t["type"] != "xhttp" {
            return Err("XTLS Vision с транспортом".into());
        }
        o["transport"] = t;
    }
    if !s(&stream["sockopt"]["dialerProxy"]).is_empty() {
        o["detour"] = json!(s(&stream["sockopt"]["dialerProxy"]));
    }
    Ok(o)
}

fn is_proxy(ob: &Value) -> bool {
    !matches!(s(&ob["protocol"]), "freedom" | "blackhole" | "dns" | "loopback")
}

/// What sing-box with our fork cannot do for this server; such servers are run by Xray.
pub fn needs_xray(server: &Server) -> Option<String> {
    match (&server.config, &server.outbound) {
        (Some(c), _) => full(c, &Settings::default(), &Local { tun: false, clash_port: 0, probe_port: 0, secret: "", rules_dir: Path::new("") }).err(),
        (None, Some(o)) => outbound(o, "proxy").err(),
        _ => Some("у сервера нет конфигурации".into()),
    }
}

/// The server's transport is XHTTP (stored link or the first proxy of a full config).
pub fn is_xhttp(server: &Server) -> bool {
    let ob = match (&server.config, &server.outbound) {
        (Some(c), _) => c["outbounds"].as_array().and_then(|a| a.iter().find(|o| is_proxy(o))).cloned(),
        (None, Some(o)) => Some(o.clone()),
        _ => None,
    };
    ob.is_some_and(|o| stream_network(&o["streamSettings"]) == "xhttp")
}

// ---------------------------------------------------------------- rules

fn rule_set_entry(name: &str, dir: &Path) -> Value {
    json!({"type": "local", "tag": name, "format": "binary", "path": dir.join(format!("{name}.srs")).to_string_lossy()})
}

/// Collects the rule-sets a config refers to.
struct Sets<'a> {
    dir: &'a Path,
    used: Vec<String>,
}

impl Sets<'_> {
    fn want(&mut self, name: String, notes: &mut Vec<String>) -> Option<String> {
        if !SHIPPED_RULE_SETS.contains(&name.as_str()) {
            notes.push(format!("правило {name} пропущено: такого набора в приложении нет"));
            return None;
        }
        if !self.dir.as_os_str().is_empty() && !self.dir.join(format!("{name}.srs")).is_file() {
            notes.push(format!("правило {name} пропущено: нет файла набора"));
            return None;
        }
        if !self.used.contains(&name) {
            self.used.push(name.clone());
        }
        Some(name)
    }
}

/// Xray domain matchers → sing-box rule fields (added to `m`). False if nothing usable.
fn domains(list: &[String], m: &mut Map<String, Value>, sets: &mut Sets, notes: &mut Vec<String>) -> bool {
    let add = |m: &mut Map<String, Value>, key: &str, v: String| {
        let e = m.entry(key).or_insert_with(|| json!([]));
        e.as_array_mut().unwrap().push(json!(v));
    };
    let mut any = false;
    for d in list {
        let d = d.trim();
        if d.is_empty() {
            continue;
        }
        let (kind, value) = match d.split_once(':') {
            Some((k, v)) if ["domain", "full", "keyword", "regexp", "geosite", "ext", "dotless"].contains(&k) => (k, v),
            _ => ("plain", d),
        };
        match kind {
            "domain" => add(m, "domain_suffix", value.trim_start_matches("*.").into()),
            "full" => add(m, "domain", value.into()),
            "keyword" | "plain" => add(m, "domain_keyword", value.into()),
            "regexp" => add(m, "domain_regex", value.into()),
            "geosite" => {
                let cat = value.split('@').next().unwrap_or(value).to_lowercase();
                match sets.want(format!("geosite-{cat}"), notes) {
                    Some(set) => add(m, "rule_set", set),
                    None => continue,
                }
            }
            _ => {
                notes.push(format!("правило {d} пропущено"));
                continue;
            }
        }
        any = true;
    }
    any
}

fn ips(list: &[String], m: &mut Map<String, Value>, sets: &mut Sets, notes: &mut Vec<String>) -> bool {
    let mut any = false;
    for ip in list.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
        if let Some(cat) = ip.strip_prefix("geoip:") {
            if cat.eq_ignore_ascii_case("private") {
                m.insert("ip_is_private".into(), json!(true));
            } else {
                match sets.want(format!("geoip-{}", cat.to_lowercase()), notes) {
                    Some(set) => {
                        m.entry("rule_set").or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!(set));
                    }
                    None => continue,
                }
            }
        } else if ip.starts_with('!') || ip.contains(':') && ip.starts_with("ext") {
            notes.push(format!("правило {ip} пропущено"));
            continue;
        } else {
            m.entry("ip_cidr").or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!(ip));
        }
        any = true;
    }
    any
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from).or_else(|| x.as_u64().map(|n| n.to_string()))).collect(),
        Value::String(t) => t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        Value::Number(n) => vec![n.to_string()],
        _ => vec![],
    }
}

/// "53", "1000-2000", "80,443" → port / port_range.
fn ports(v: &Value, m: &mut Map<String, Value>, prefix: &str) -> bool {
    let (mut single, mut ranges) = (vec![], vec![]);
    for p in strings(v) {
        match p.split_once('-') {
            Some((a, b)) => ranges.push(json!(format!("{}:{}", a.trim(), b.trim()))),
            None => {
                if let Ok(n) = p.parse::<u16>() {
                    single.push(json!(n));
                }
            }
        }
    }
    if !single.is_empty() {
        m.insert(format!("{prefix}port"), json!(single));
    }
    if !ranges.is_empty() {
        m.insert(format!("{prefix}port_range"), json!(ranges));
    }
    !single.is_empty() || !ranges.is_empty()
}

fn route_to(mut m: Map<String, Value>, target: &Target) -> Value {
    match target {
        Target::Out(tag) => {
            m.insert("outbound".into(), json!(tag));
        }
        Target::Reject => {
            m.insert("action".into(), json!("reject"));
        }
        Target::Dns => {
            m.insert("action".into(), json!("hijack-dns"));
        }
    }
    Value::Object(m)
}

enum Target {
    Out(String),
    Reject,
    Dns,
}

/// The user's own rules.
fn user_rules(st: &Settings, proxy: &str, direct: &str, sets: &mut Sets, notes: &mut Vec<String>) -> Vec<Value> {
    let mut out = vec![];
    for r in &st.rules {
        let value = r.value.trim();
        if value.is_empty() {
            continue;
        }
        let target = match r.action.as_str() {
            "direct" => Target::Out(direct.into()),
            "block" => Target::Reject,
            _ => Target::Out(proxy.into()),
        };
        let mut m = Map::new();
        let ok = if r.kind == "ip" {
            ips(&[value.to_string()], &mut m, sets, notes)
        } else {
            let v = if value.contains(':') { value.to_string() } else { format!("domain:{value}") };
            domains(&[v], &mut m, sets, notes)
        };
        if ok {
            out.push(route_to(m, &target));
        }
    }
    out
}

fn cleaned(list: &[String]) -> Vec<String> {
    list.iter().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
}

/// The exclusions: rules before everything else and rules after everything (see config.rs).
fn bypass_rules(st: &Settings, proxy: &str, direct: &str, sets: &mut Sets, notes: &mut Vec<String>) -> (Vec<Value>, Vec<Value>) {
    let apps = if cfg!(target_os = "android") { vec![] } else { cleaned(&st.bypass_apps) };
    let doms: Vec<String> = cleaned(&st.bypass_domains).into_iter()
        .map(|d| if d.contains(':') { d } else { format!("domain:{}", d.trim_start_matches("*.")) }).collect();
    let only = st.bypass_mode == "only";
    if only && apps.is_empty() && doms.is_empty() {
        return (vec![], vec![]);
    }
    if only && cfg!(target_os = "android") && !cleaned(&st.bypass_apps).is_empty() {
        return (vec![], vec![]);
    }
    let target = Target::Out(if only { proxy } else { direct }.into());
    let mut first = vec![];
    if !apps.is_empty() {
        let mut m = Map::new();
        m.insert("process_name".into(), json!(apps));
        first.push(route_to(m, &target));
    }
    let mut m = Map::new();
    if domains(&doms, &mut m, sets, notes) {
        first.push(route_to(m, &target));
    }
    let last = if only { vec![json!({"network": ["tcp", "udp"], "outbound": direct})] } else { vec![] };
    (first, last)
}

// ---------------------------------------------------------------- whole config

fn dns_servers(list: &str, detour: &str) -> Vec<Value> {
    let mut out = vec![];
    for (i, d) in list.split([',', ' ', '\n']).map(str::trim).filter(|d| !d.is_empty()).enumerate() {
        let tag = format!("dns-{i}");
        let server = if d == "localhost" {
            json!({"type": "local", "tag": tag})
        } else if let Some(rest) = d.strip_prefix("https://") {
            let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/dns-query".into()));
            json!({"type": "https", "tag": tag, "server": host, "path": path, "detour": detour})
        } else if let Some(host) = d.strip_prefix("tls://") {
            json!({"type": "tls", "tag": tag, "server": host, "detour": detour})
        } else if let Some(host) = d.strip_prefix("tcp://") {
            json!({"type": "tcp", "tag": tag, "server": host, "detour": detour})
        } else {
            let host = d.strip_prefix("udp://").unwrap_or(d);
            json!({"type": "udp", "tag": tag, "server": host, "detour": detour})
        };
        out.push(server);
    }
    if out.is_empty() {
        out = dns_servers("1.1.1.1, 8.8.8.8", detour);
    }
    out
}

fn dns(list: &str, detour: &str, strategy: Option<&str>) -> Value {
    let mut servers = dns_servers(list, detour);
    let first = s(&servers[0]["tag"]).to_string();
    // names of servers themselves (when not pinned beforehand) go straight to the system
    servers.push(json!({"type": "local", "tag": "dns-local"}));
    let mut d = json!({"servers": servers, "final": first});
    if let Some(st) = strategy {
        d["strategy"] = json!(st);
    }
    d
}

fn inbounds(st: &Settings, tun: bool) -> Vec<Value> {
    let mut list = vec![
        json!({"type": "socks", "tag": "socks", "listen": listen(st), "listen_port": st.socks_port}),
        json!({"type": "http", "tag": "http", "listen": listen(st), "listen_port": st.http_port}),
    ];
    if tun {
        let mut t = json!({"type": "tun", "tag": TUN_TAG, "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"], "mtu": 1500,
            "auto_route": true, "strict_route": true, "stack": "mixed"});
        if let Some(name) = sys::TUN_NAME {
            t["interface_name"] = json!(name);
        }
        list.push(t);
    }
    list
}

fn head_rules(tun: bool) -> Vec<Value> {
    let mut r = vec![json!({"action": "sniff"})];
    if tun {
        r.push(json!({"inbound": [TUN_TAG], "port": [53], "action": "hijack-dns"}));
        r.push(json!({"protocol": "dns", "action": "hijack-dns"}));
    }
    r
}

fn finish(st: &Settings, local: &Local, mut inbounds: Vec<Value>, outbounds: Vec<Value>, mut rules: Vec<Value>, final_tag: &str,
          dns: Value, sets: Sets) -> Value {
    // the check's own port, straight to the server (see Local::probe_port); not when the
    // default way out is direct, where it would prove nothing
    let final_is_server = outbounds.iter().any(|o| s(&o["tag"]) == final_tag && !matches!(s(&o["type"]), "direct" | "block"));
    if local.probe_port != 0 && final_is_server {
        inbounds.push(json!({"type": "socks", "tag": PROBE_TAG, "listen": "127.0.0.1", "listen_port": local.probe_port}));
        rules.insert(0, json!({"inbound": [PROBE_TAG], "outbound": final_tag}));
    }
    let mut route = json!({"rules": rules, "final": final_tag, "default_domain_resolver": "dns-local"});
    if local.tun {
        route["auto_detect_interface"] = json!(true);
    }
    if !sets.used.is_empty() {
        route["rule_set"] = json!(sets.used.iter().map(|n| rule_set_entry(n, sets.dir)).collect::<Vec<_>>());
    }
    let level = match st.log_level.as_str() {
        "debug" => "debug",
        "error" => "error",
        "none" => "error",
        // info internally: the "started" line is at info; noise is filtered by core.rs
        _ => "info",
    };
    json!({
        "log": {"level": level, "timestamp": false, "disabled": false},
        "dns": dns,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": route,
        "experimental": {"clash_api": {"external_controller": format!("127.0.0.1:{}", local.clash_port), "secret": local.secret}},
    })
}

/// Our rules around a proxy tag (links and the auto group).
fn link_rules(st: &Settings, local: &Local, proxy: &str, sets: &mut Sets, notes: &mut Vec<String>) -> Vec<Value> {
    let mut rules = head_rules(local.tun);
    let (first, last) = bypass_rules(st, proxy, "direct", sets, notes);
    rules.extend(first);
    rules.extend(user_rules(st, proxy, "direct", sets, notes));
    // always direct, every routing mode included (0.3.2): the home network stays reachable
    rules.push(json!({"ip_cidr": PRIVATE_NETWORKS, "outbound": "direct"}));
    if st.routing == "ru" {
        let mut m = Map::new();
        domains(&["geosite:category-ru".into(), "domain:ru".into(), "domain:su".into(), "domain:xn--p1ai".into()], &mut m, sets, notes);
        rules.push(route_to(m, &Target::Out("direct".into())));
        let mut m = Map::new();
        if ips(&["geoip:ru".into()], &mut m, sets, notes) {
            // Through the ports and the system proxy names arrive as names, and matching them
            // against geoip meant a DNS lookup through the server before every new site: UDP
            // over a fresh connection, seconds on a lossy path, and a failed request when it
            // got lost (0.3.2). There the names now go to the server as they are, and only
            // addresses are matched by country. The all-traffic mode sees addresses anyway.
            if local.tun {
                rules.push(json!({"action": "resolve", "strategy": "ipv4_only"}));
            }
            rules.push(route_to(m, &Target::Out("direct".into())));
        }
    }
    rules.extend(last);
    rules
}

fn from_link(ob: &Value, st: &Settings, local: &Local) -> Result<Built, String> {
    let proxy = outbound(ob, "proxy")?;
    let mut notes = vec![];
    let mut sets = Sets { dir: local.rules_dir, used: vec![] };
    let rules = link_rules(st, local, "proxy", &mut sets, &mut notes);
    let outbounds = vec![proxy, json!({"type": "direct", "tag": "direct"})];
    let config = finish(st, local, inbounds(st, local.tun), outbounds, rules, "proxy", dns(&st.dns, "proxy", Some("ipv4_only")), sets);
    Ok(Built { config, notes, members: HashMap::new() })
}

/// Tags that are ours in a translated provider config.
fn safe_tag(tag: &str, taken: &HashSet<String>) -> String {
    let base = if tag.is_empty() { "out".to_string() } else { tag.to_string() };
    if !taken.contains(&base) {
        return base;
    }
    (1..).map(|n| format!("{base}-{n}")).find(|t| !taken.contains(t)).unwrap()
}

/// A provider's whole Xray config: its outbounds, balancers and routing; our local side.
fn full(cfg: &Value, st: &Settings, local: &Local) -> Result<Built, String> {
    let mut notes = vec![];
    let mut sets = Sets { dir: local.rules_dir, used: vec![] };
    let obs = cfg["outbounds"].as_array().ok_or("в конфигурации нет исходящих")?;
    let mut taken: HashSet<String> = HashSet::new();
    let mut map: HashMap<String, Target> = HashMap::new();
    let mut outbounds = vec![];
    let mut first_out: Option<String> = None;
    let mut direct: Option<String> = None;
    let mut fragment_tags: HashSet<String> = HashSet::new();
    for (i, ob) in obs.iter().enumerate() {
        let tag = s(&ob["tag"]).to_string();
        let tag = if tag.is_empty() { format!("out-{i}") } else { tag };
        match s(&ob["protocol"]) {
            "freedom" => {
                if !ob["settings"]["noises"].is_null() {
                    return Err("шумы (noises) в исходящем freedom".into());
                }
                if !s(&ob["settings"]["redirect"]).is_empty() {
                    return Err("перенаправление в исходящем freedom".into());
                }
                if ob["settings"]["fragment"].is_object() {
                    fragment_tags.insert(tag.clone());
                }
                let t = safe_tag(&tag, &taken);
                taken.insert(t.clone());
                outbounds.push(json!({"type": "direct", "tag": t}));
                direct.get_or_insert(t.clone());
                first_out.get_or_insert(t.clone());
                map.insert(tag, Target::Out(t));
            }
            "blackhole" => {
                map.insert(tag, Target::Reject);
            }
            "dns" => {
                map.insert(tag, Target::Dns);
            }
            "loopback" => return Err("исходящий loopback".into()),
            _ => {
                let t = safe_tag(&tag, &taken);
                taken.insert(t.clone());
                let o = outbound(ob, &t).map_err(|e| format!("{tag}: {e}"))?;
                outbounds.push(o);
                first_out.get_or_insert(t.clone());
                map.insert(tag, Target::Out(t));
            }
        }
    }
    // dialerProxy: tags are the provider's; a chain through a "fragment" freedom becomes TLS
    // fragmentation of the outbound itself.
    for o in outbounds.iter_mut() {
        let Some(via) = o["detour"].as_str().map(String::from) else { continue };
        if fragment_tags.contains(&via) {
            o.as_object_mut().unwrap().remove("detour");
            if o["tls"].is_object() {
                o["tls"]["fragment"] = json!(true);
            }
            continue;
        }
        match map.get(&via) {
            Some(Target::Out(t)) => o["detour"] = json!(t),
            _ => return Err(format!("цепочка через {via}")),
        }
    }
    let direct = match direct {
        Some(d) => d,
        None => {
            let t = safe_tag("direct", &taken);
            taken.insert(t.clone());
            outbounds.push(json!({"type": "direct", "tag": t}));
            t
        }
    };

    // balancers → urltest groups
    let routing = &cfg["routing"];
    let probe = cfg["observatory"]["probeURL"].as_str().or(cfg["observatory"]["probeUrl"].as_str())
        .or(cfg["burstObservatory"]["pingConfig"]["destination"].as_str()).unwrap_or(PROBE_URL).to_string();
    let interval = cfg["observatory"]["probeInterval"].as_str().or(cfg["burstObservatory"]["pingConfig"]["interval"].as_str())
        .unwrap_or("3m").to_string();
    let mut balancers: HashMap<String, String> = HashMap::new();
    for b in routing["balancers"].as_array().into_iter().flatten() {
        let btag = s(&b["tag"]).to_string();
        let prefixes = strings(&b["selector"]);
        let members: Vec<String> = obs.iter().filter(|o| is_proxy(o))
            .filter(|o| prefixes.iter().any(|p| s(&o["tag"]).starts_with(p.as_str())))
            .filter_map(|o| match map.get(s(&o["tag"])) { Some(Target::Out(t)) => Some(t.clone()), _ => None }).collect();
        let fallback = match map.get(s(&b["fallbackTag"])) { Some(Target::Out(t)) => Some(t.clone()), _ => None };
        let t = safe_tag(&format!("balancer-{btag}"), &taken);
        taken.insert(t.clone());
        if members.is_empty() {
            match fallback {
                Some(f) => {
                    balancers.insert(btag, f);
                    continue;
                }
                None => return Err(format!("балансировщик {btag} без серверов")),
            }
        }
        let group = match s(&b["strategy"]["type"]) {
            "random" | "roundRobin" => json!({"type": "selector", "tag": t, "outbounds": members}),
            _ => json!({"type": "urltest", "tag": t, "outbounds": members, "url": probe, "interval": interval, "tolerance": 50}),
        };
        outbounds.push(group);
        balancers.insert(btag, t);
    }

    // inbound tags of the provider's local ports are ours now
    let local_in: HashSet<String> = cfg["inbounds"].as_array().into_iter().flatten()
        .filter(|i| matches!(s(&i["protocol"]), "socks" | "mixed" | "http")).map(|i| s(&i["tag"]).to_string()).collect();
    let resolve_first = matches!(s(&routing["domainStrategy"]), "IPIfNonMatch" | "IPOnDemand");

    let mut rules = head_rules(local.tun);
    let proxy = first_out.clone().unwrap_or(direct.clone());
    let main_proxy = outbounds.iter().find(|o| !matches!(s(&o["type"]), "direct" | "urltest" | "selector"))
        .map(|o| s(&o["tag"]).to_string()).unwrap_or(proxy.clone());
    let (first, last) = bypass_rules(st, &main_proxy, &direct, &mut sets, &mut notes);
    rules.extend(first);
    rules.extend(user_rules(st, &main_proxy, &direct, &mut sets, &mut notes));
    rules.extend(last);
    let mut resolved = false;
    for r in routing["rules"].as_array().into_iter().flatten() {
        if !r["type"].is_null() && s(&r["type"]) != "field" {
            continue;
        }
        let target = if let Some(b) = r["balancerTag"].as_str() {
            match balancers.get(b) {
                Some(t) => Target::Out(t.clone()),
                None => return Err(format!("правило ведёт на неизвестный балансировщик {b}")),
            }
        } else {
            match map.get(s(&r["outboundTag"])) {
                Some(Target::Out(t)) => Target::Out(t.clone()),
                Some(Target::Reject) => Target::Reject,
                Some(Target::Dns) => Target::Dns,
                None => return Err(format!("правило ведёт на неизвестный исходящий {}", s(&r["outboundTag"]))),
            }
        };
        if ["user", "attrs", "process", "vlessRoute", "webhook"].iter().any(|k| !r[*k].is_null()) {
            notes.push("правило провайдера с условиями, которых нет в sing-box, пропущено".into());
            continue;
        }
        let mut dm = Map::new();
        let mut im = Map::new();
        let mut m = Map::new();
        let has_domain = !r["domain"].is_null() || !r["domains"].is_null();
        let has_ip = !r["ip"].is_null();
        let d_ok = has_domain && domains(&strings(if r["domain"].is_null() { &r["domains"] } else { &r["domain"] }), &mut dm, &mut sets, &mut notes);
        let i_ok = has_ip && ips(&strings(&r["ip"]), &mut im, &mut sets, &mut notes);
        if (has_domain && !d_ok) || (has_ip && !i_ok) {
            continue; // every matcher of that kind was dropped: the rule cannot match as meant
        }
        let mut cond = false;
        if !r["port"].is_null() {
            cond |= ports(&r["port"], &mut m, "");
        }
        if !r["sourcePort"].is_null() {
            cond |= ports(&r["sourcePort"], &mut m, "source_");
        }
        if let Some(src) = r["source"].as_array().or(r["sourceIP"].as_array()) {
            m.insert("source_ip_cidr".into(), json!(src.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()));
            cond = true;
        }
        let nets = strings(&r["network"]);
        if !nets.is_empty() {
            m.insert("network".into(), json!(nets));
            cond = true;
        }
        let protos = strings(&r["protocol"]);
        if !protos.is_empty() {
            m.insert("protocol".into(), json!(protos));
            cond = true;
        }
        if !r["inboundTag"].is_null() {
            let wanted = strings(&r["inboundTag"]);
            if !wanted.iter().any(|t| local_in.contains(t)) {
                continue; // an inbound we do not create: never matches
            }
            let mut ours = vec!["socks", "http"];
            if local.tun {
                ours.push(TUN_TAG);
            }
            m.insert("inbound".into(), json!(ours));
            cond = true;
        }
        if resolve_first && i_ok && !resolved {
            // IPv4 unless the provider asks for IPv6: most servers cannot reach IPv6 at all
            let family = if s(&cfg["dns"]["queryStrategy"]) == "UseIPv6" { "ipv6_only" } else { "ipv4_only" };
            rules.push(json!({"action": "resolve", "strategy": family}));
            resolved = true;
        }
        if d_ok && i_ok {
            // Xray wants both; sing-box would accept either
            let mut and = vec![Value::Object(dm), Value::Object(im)];
            if cond {
                and.push(Value::Object(m.clone()));
            }
            let mut lm = Map::new();
            lm.insert("type".into(), json!("logical"));
            lm.insert("mode".into(), json!("and"));
            lm.insert("rules".into(), json!(and));
            rules.push(route_to(lm, &target));
            continue;
        }
        m.extend(dm);
        m.extend(im);
        if !cond && !d_ok && !i_ok {
            m.insert("network".into(), json!(["tcp", "udp"]));
        }
        rules.push(route_to(m, &target));
    }

    let strategy = match s(&cfg["dns"]["queryStrategy"]) {
        "UseIPv4" => Some("ipv4_only"),
        "UseIPv6" => Some("ipv6_only"),
        _ => None,
    };
    let dns_list: Vec<String> = cfg["dns"]["servers"].as_array().into_iter().flatten()
        .filter_map(|d| d.as_str().map(String::from).or_else(|| d["address"].as_str().map(String::from)))
        .map(|d| if d.starts_with("https+local://") { d.replacen("https+local://", "https://", 1) } else { d })
        .filter(|d| !d.starts_with("fakedns") && !d.starts_with("quic"))
        .collect();
    let dns_text = if dns_list.is_empty() { st.dns.clone() } else { dns_list.join(",") };
    let final_tag = proxy;
    let config = finish(st, local, inbounds(st, local.tun), outbounds, rules, &final_tag, dns(&dns_text, &main_proxy, strategy), sets);
    Ok(Built { config, notes, members: HashMap::new() })
}

/// The config for one server.
pub fn build(server: &Server, st: &Settings, local: &Local) -> Result<Built, String> {
    match (&server.config, &server.outbound) {
        (Some(c), _) => full(c, st, local),
        (None, Some(o)) => from_link(o, st, local),
        _ => Err("у сервера нет конфигурации".into()),
    }
}

/// The proxy outbound that stands for a server in a group: the link's, or the first proxy of
/// a provider's config (its routing is not used inside the group).
pub fn primary(server: &Server) -> Option<Value> {
    match (&server.config, &server.outbound) {
        (Some(c), _) => c["outbounds"].as_array()?.iter().find(|o| is_proxy(o)).cloned(),
        (None, Some(o)) => Some(o.clone()),
        _ => None,
    }
}

/// "Авто, самый быстрый": every given server in a urltest group that sing-box keeps pointed
/// at the fastest one that works.
pub fn build_auto(servers: &[Server], st: &Settings, local: &Local) -> Result<Built, String> {
    let mut outbounds = vec![];
    let mut members = HashMap::new();
    let mut tags = vec![];
    for (i, server) in servers.iter().enumerate() {
        let Some(ob) = primary(server) else { continue };
        let tag = format!("s{i}");
        // a provider config's chains are left out of the group
        let mut ob = ob;
        if ob["streamSettings"]["sockopt"]["dialerProxy"].is_string() {
            continue;
        }
        ob["tag"] = json!(tag);
        let Ok(o) = outbound(&ob, &tag) else { continue };
        outbounds.push(o);
        members.insert(tag.clone(), server.id.clone());
        tags.push(tag);
    }
    if tags.is_empty() {
        return Err("нет серверов, которые можно проверить автоматически".into());
    }
    outbounds.insert(0, json!({"type": "urltest", "tag": AUTO_TAG, "outbounds": tags, "url": PROBE_URL, "interval": "3m",
        "tolerance": 50, "idle_timeout": "30m"}));
    outbounds.push(json!({"type": "direct", "tag": "direct"}));
    let mut notes = vec![];
    let mut sets = Sets { dir: local.rules_dir, used: vec![] };
    let rules = link_rules(st, local, AUTO_TAG, &mut sets, &mut notes);
    let config = finish(st, local, inbounds(st, local.tun), outbounds, rules, AUTO_TAG, dns(&st.dns, AUTO_TAG, Some("ipv4_only")), sets);
    Ok(Built { config, notes, members })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Rule;

    fn local(tun: bool) -> Local<'static> {
        Local { tun, clash_port: 9090, probe_port: 0, secret: "x", rules_dir: Path::new("") }
    }

    fn link(uri: &str) -> Server {
        crate::links::parse(uri).unwrap()
    }

    fn rules(cfg: &Value) -> Vec<Value> {
        cfg["route"]["rules"].as_array().unwrap().clone()
    }

    #[test]
    fn the_check_has_its_own_port_straight_to_the_server() {
        let srv = link("vless://11111111-2222-3333-4444-555555555555@198.51.100.7:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=www.site.com&fp=firefox&pbk=PUBKEY&sid=ab12#x");
        let st = Settings { routing: "ru".into(), bypass_domains: vec!["example.org".into()], ..Default::default() };
        let l = Local { tun: true, clash_port: 9090, probe_port: 31999, secret: "x", rules_dir: Path::new("") };
        let c = build(&srv, &st, &l).unwrap().config;
        let probe = c["inbounds"].as_array().unwrap().iter().find(|i| i["tag"] == PROBE_TAG).unwrap();
        assert_eq!(probe["listen"], "127.0.0.1");
        assert_eq!(probe["listen_port"], 31999);
        // first rule: before sniffing, exclusions, the Russia preset and its DNS lookup
        assert_eq!(rules(&c)[0], json!({"inbound": [PROBE_TAG], "outbound": "proxy"}));
        // the lookup the preset needs asks for IPv4 only, and so does the DNS
        assert!(rules(&c).iter().any(|r| *r == json!({"action": "resolve", "strategy": "ipv4_only"})));
        assert_eq!(c["dns"]["strategy"], "ipv4_only");
        // no check port asked for: none made; and through the ports no lookup before every site
        let c = build(&srv, &st, &local(false)).unwrap().config;
        assert!(!c["inbounds"].as_array().unwrap().iter().any(|i| i["tag"] == PROBE_TAG));
        assert!(!rules(&c).iter().any(|r| r["action"] == "resolve"));
    }

    #[test]
    fn reality_vision_link() {
        let srv = link("vless://11111111-2222-3333-4444-555555555555@198.51.100.7:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=www.site.com&fp=firefox&pbk=PUBKEY&sid=ab12&spx=%2F#x");
        assert!(needs_xray(&srv).is_none());
        let b = build(&srv, &Settings::default(), &local(false)).unwrap();
        let p = &b.config["outbounds"][0];
        assert_eq!(p["type"], "vless");
        assert_eq!(p["flow"], "xtls-rprx-vision");
        assert_eq!(p["server"], "198.51.100.7");
        assert_eq!(p["tls"]["server_name"], "www.site.com");
        assert_eq!(p["tls"]["utls"]["fingerprint"], "firefox");
        assert_eq!(p["tls"]["reality"], json!({"enabled": true, "public_key": "PUBKEY", "short_id": "ab12"}));
        assert!(p.get("transport").is_none());
        assert_eq!(b.config["route"]["final"], "proxy");
        assert_eq!(b.config["inbounds"][0]["listen_port"], 20808);
        assert_eq!(b.config["inbounds"][1]["type"], "http");
        assert_eq!(b.config["experimental"]["clash_api"]["external_controller"], "127.0.0.1:9090");
        // the home network stays direct in every mode
        assert!(rules(&b.config).iter().any(|r| r["ip_cidr"] == json!(PRIVATE_NETWORKS) && r["outbound"] == "direct"));
    }

    #[test]
    fn ws_tls_link_with_early_data() {
        let srv = link("vless://u@h.example:443?type=ws&security=tls&path=%2Fws%3Fed%3D2048&host=cdn.example&fp=chrome&alpn=http%2F1.1#w");
        let b = build(&srv, &Settings::default(), &local(false)).unwrap();
        let p = &b.config["outbounds"][0];
        assert_eq!(p["transport"]["type"], "ws");
        assert_eq!(p["transport"]["path"], "/ws");
        assert_eq!(p["transport"]["max_early_data"], 2048);
        assert_eq!(p["transport"]["headers"]["Host"], "cdn.example");
        assert_eq!(p["tls"]["server_name"], "cdn.example");
        assert_eq!(p["tls"]["alpn"], json!(["http/1.1"]));
    }

    #[test]
    fn xhttp_with_extra_and_xmux() {
        let extra = r#"{"xPaddingBytes":"100-1000","noGRPCHeader":true,"scMaxEachPostBytes":500000,"headers":{"X-A":"1","Host":"no"},"xmux":{"maxConcurrency":"16-32","hMaxRequestTimes":"600-900","hKeepAlivePeriod":10}}"#;
        let uri = format!("vless://u@203.0.113.5:443?type=xhttp&security=tls&sni=a.b&path=%2Fx&host=a.b&mode=packet-up&extra={}#x",
            percent_encoding::utf8_percent_encode(extra, percent_encoding::NON_ALPHANUMERIC));
        let srv = link(&uri);
        assert!(is_xhttp(&srv));
        let b = build(&srv, &Settings::default(), &local(false)).unwrap();
        let t = &b.config["outbounds"][0]["transport"];
        assert_eq!(t["type"], "xhttp");
        assert_eq!(t["mode"], "packet-up");
        assert_eq!(t["path"], "/x");
        assert_eq!(t["host"], "a.b");
        assert_eq!(t["x_padding_bytes"], "100-1000");
        assert_eq!(t["no_grpc_header"], true);
        assert_eq!(t["sc_max_each_post_bytes"], "500000");
        assert_eq!(t["headers"], json!({"X-A": "1"}));
        assert_eq!(t["xmux"], json!({"max_concurrency": "16-32", "h_max_request_times": "600-900", "h_keep_alive_period": 10}));
        assert_eq!(b.config["outbounds"][0]["tls"]["alpn"], json!(["h2", "http/1.1"]));
        // REALITY under XHTTP, mode auto
        let r = link("vless://u@198.51.100.9:443?type=xhttp&security=reality&sni=www.nvidia.com&pbk=K&sid=1&path=%2Fp&mode=auto#r");
        let b = build(&r, &Settings::default(), &local(false)).unwrap();
        assert_eq!(b.config["outbounds"][0]["transport"], json!({"type": "xhttp", "path": "/p", "mode": "auto"}));
        assert_eq!(b.config["outbounds"][0]["tls"]["reality"]["public_key"], "K");
        // what stays with Xray
        let down = link(&format!("vless://u@198.51.100.9:443?type=xhttp&security=tls&extra={}#d",
            percent_encoding::utf8_percent_encode(r#"{"downloadSettings":{"address":"x"}}"#, percent_encoding::NON_ALPHANUMERIC)));
        assert!(needs_xray(&down).unwrap().contains("downloadSettings"));
        let h3 = link("vless://u@198.51.100.9:443?type=xhttp&security=tls&alpn=h3#h");
        assert!(needs_xray(&h3).is_some());
    }

    #[test]
    fn grpc_and_unsupported() {
        let g = link("vless://u@198.51.100.1:2053?type=grpc&serviceName=svc&security=reality&sni=dzen.ru&pbk=K&sid=0d#g");
        let b = build(&g, &Settings::default(), &local(false)).unwrap();
        assert_eq!(b.config["outbounds"][0]["transport"], json!({"type": "grpc", "service_name": "svc"}));
        assert!(needs_xray(&link("vless://u@h:1?type=kcp#k")).is_some());
        assert!(needs_xray(&link("vless://u@h:1?type=tcp&headerType=http&host=a#t")).is_some());
        assert!(needs_xray(&link("vless://u@h:1?type=tcp&encryption=mlkem768x25519plus.native.0rtt.x#e")).is_some());
        assert!(needs_xray(&link("vless://u@h:1?type=tcp&security=reality&pbk=K&pqv=abc#p")).is_some());
        assert!(needs_xray(&link("trojan://p@h:443?security=tls&sni=h#t")).is_none());
        assert!(needs_xray(&link("ss://Y2hhY2hhMjAtaWV0Zi1wb2x5MTMwNTpwYXNz@1.2.3.4:8388#n")).is_none());
    }

    #[test]
    fn exclusions_come_first_and_go_direct() {
        let st = Settings { bypass_apps: vec!["Steam.exe".into(), " ".into()], bypass_domains: vec!["*.example.org".into(), "geosite:google".into()], ..Default::default() };
        let srv = link("vless://u@1.2.3.4:443?type=tcp#a");
        let b = build(&srv, &st, &local(false)).unwrap();
        let r = rules(&b.config);
        assert_eq!(r[0], json!({"action": "sniff"}));
        assert_eq!(r[1], json!({"process_name": ["Steam.exe"], "outbound": "direct"}));
        // geosite:google is not shipped: dropped with a note, the rest stays
        assert_eq!(r[2], json!({"domain_suffix": ["example.org"], "outbound": "direct"}));
        assert!(b.notes.iter().any(|n| n.contains("geosite-google")));
    }

    #[test]
    fn only_mode_sends_the_listed_through_the_server_and_the_rest_past_it() {
        let st = Settings { bypass_mode: "only".into(), bypass_apps: vec!["Steam.exe".into()], bypass_domains: vec!["example.org".into()], ..Default::default() };
        let srv = link("vless://u@1.2.3.4:443?type=tcp#a");
        let r = rules(&build(&srv, &st, &local(false)).unwrap().config);
        assert_eq!(r[1], json!({"process_name": ["Steam.exe"], "outbound": "proxy"}));
        assert_eq!(r[2]["outbound"], "proxy");
        assert_eq!(r.last().unwrap(), &json!({"network": ["tcp", "udp"], "outbound": "direct"}));
        let empty = Settings { bypass_mode: "only".into(), ..Default::default() };
        let r = rules(&build(&srv, &empty, &local(false)).unwrap().config);
        assert!(r.iter().all(|x| x.get("network").is_none()));
    }

    #[test]
    fn user_rules_and_ru_preset() {
        let dir = std::env::temp_dir().join(format!("quadra-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in SHIPPED_RULE_SETS {
            std::fs::write(dir.join(format!("{n}.srs")), b"x").unwrap();
        }
        let mut st = Settings { routing: "ru".into(), ..Default::default() };
        st.rules.push(Rule { kind: "domain".into(), value: "example.org".into(), action: "block".into() });
        st.rules.push(Rule { kind: "ip".into(), value: "203.0.113.0/24".into(), action: "direct".into() });
        let srv = link("vless://u@1.2.3.4:443?type=tcp#a");
        let l = Local { tun: true, clash_port: 1, probe_port: 0, secret: "s", rules_dir: &dir };
        let b = build(&srv, &st, &l).unwrap();
        let r = rules(&b.config);
        assert_eq!(r[1], json!({"inbound": ["tun"], "port": [53], "action": "hijack-dns"}));
        assert!(r.contains(&json!({"domain_suffix": ["example.org"], "action": "reject"})));
        assert!(r.contains(&json!({"ip_cidr": ["203.0.113.0/24"], "outbound": "direct"})));
        assert!(r.iter().any(|x| x["rule_set"] == json!(["geosite-category-ru"]) && x["domain_suffix"] == json!(["ru", "su", "xn--p1ai"])));
        assert!(r.iter().any(|x| x["rule_set"] == json!(["geoip-ru"])));
        assert_eq!(b.config["route"]["rule_set"].as_array().unwrap().len(), 2);
        assert_eq!(b.config["route"]["auto_detect_interface"], true);
        assert_eq!(b.config["inbounds"][2]["type"], "tun");
        assert_eq!(b.config["inbounds"][2]["auto_route"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fleet_style_full_config() {
        let cfg = json!({"dns": {"servers": ["1.1.1.1", "1.0.0.1"], "queryStrategy": "UseIP"},
            "routing": {"rules": [{"type": "field", "protocol": ["bittorrent"], "outboundTag": "direct"}],
                "domainMatcher": "hybrid", "domainStrategy": "IPIfNonMatch"},
            "inbounds": [{"tag": "socks", "port": 10808, "listen": "127.0.0.1", "protocol": "socks"},
                {"tag": "http", "port": 10809, "listen": "127.0.0.1", "protocol": "http"}],
            "outbounds": [{"tag": "proxy", "protocol": "vless", "settings": {"vnext": [{"address": "198.51.100.3", "port": 9444,
                "users": [{"id": "u", "encryption": "none", "flow": "xtls-rprx-vision"}]}]},
                "streamSettings": {"network": "tcp", "tcpSettings": {}, "security": "reality",
                    "realitySettings": {"serverName": "www.samsung.com", "publicKey": "K", "shortId": "24", "fingerprint": "firefox"}}},
                {"tag": "direct", "protocol": "freedom"}, {"tag": "block", "protocol": "blackhole"}],
            "remarks": "x"});
        let srv = Server { config: Some(cfg), ..Default::default() };
        assert!(needs_xray(&srv).is_none());
        let b = build(&srv, &Settings::default(), &local(false)).unwrap();
        let c = &b.config;
        assert_eq!(c["route"]["final"], "proxy");
        assert!(rules(c).contains(&json!({"protocol": ["bittorrent"], "outbound": "direct"})));
        assert_eq!(c["outbounds"].as_array().unwrap().len(), 2);
        assert_eq!(c["dns"]["servers"][0], json!({"type": "udp", "tag": "dns-0", "server": "1.1.1.1", "detour": "proxy"}));
        assert_eq!(c["inbounds"][0]["listen_port"], 20808);
    }

    #[test]
    fn balancer_rules_and_tun_tags() {
        let cfg = json!({
            "inbounds": [{"tag": "socks-in", "port": 1, "protocol": "socks"}, {"tag": "entry-quic", "port": 2, "protocol": "dokodemo-door"}],
            "outbounds": [
                {"tag": "eu-1", "protocol": "trojan", "settings": {"servers": [{"address": "198.51.100.1", "port": 443, "password": "p"}]}, "streamSettings": {"security": "tls"}},
                {"tag": "eu-2", "protocol": "trojan", "settings": {"servers": [{"address": "198.51.100.2", "port": 443, "password": "p"}]}, "streamSettings": {"security": "tls"}},
                {"tag": "direct", "protocol": "freedom"}, {"tag": "blocked", "protocol": "blackhole"}],
            "observatory": {"probeURL": "https://cp.cloudflare.com/generate_204", "probeInterval": "1m"},
            "routing": {"domainStrategy": "IPIfNonMatch", "balancers": [{"tag": "lb", "selector": ["eu-"], "strategy": {"type": "leastPing"}}],
                "rules": [
                    {"type": "field", "domain": ["geosite:category-ads-all"], "outboundTag": "blocked"},
                    {"type": "field", "ip": ["geoip:private"], "outboundTag": "direct"},
                    {"type": "field", "inboundTag": ["entry-quic"], "balancerTag": "lb"},
                    {"type": "field", "inboundTag": ["socks-in"], "balancerTag": "lb"},
                    {"type": "field", "port": "25,465-587", "outboundTag": "blocked"}]}});
        let srv = Server { config: Some(cfg), ..Default::default() };
        let b = build(&srv, &Settings::default(), &local(true)).unwrap();
        let c = &b.config;
        let lb = c["outbounds"].as_array().unwrap().iter().find(|o| o["type"] == "urltest").unwrap();
        assert_eq!(lb["outbounds"], json!(["eu-1", "eu-2"]));
        assert_eq!(lb["url"], "https://cp.cloudflare.com/generate_204");
        let r = rules(c);
        // ads category is not shipped: dropped; private → resolve first, then ip_is_private
        assert!(!r.iter().any(|x| x["action"] == "reject" && x.get("rule_set").is_some()));
        let pos = r.iter().position(|x| x["action"] == "resolve").unwrap();
        assert_eq!(r[pos + 1], json!({"ip_is_private": true, "outbound": "direct"}));
        // the dokodemo inbound is gone, the socks one now means our socks, http and tun
        assert_eq!(r.iter().filter(|x| x.get("inbound").is_some() && x["outbound"] == lb["tag"]).count(), 1);
        assert!(r.iter().any(|x| x["inbound"] == json!(["socks", "http", "tun"])));
        assert!(r.contains(&json!({"port": [25], "port_range": ["465:587"], "action": "reject"})));
        assert_eq!(c["route"]["final"], "eu-1");
    }

    #[test]
    fn auto_group() {
        let a = link("vless://u@198.51.100.1:443?type=tcp&security=reality&pbk=K&sid=1&sni=a#a");
        let b = link("trojan://p@198.51.100.2:443?security=tls&sni=b#b");
        let k = link("vless://u@198.51.100.3:443?type=kcp#k");
        let servers = vec![Server { id: "A".into(), ..a }, Server { id: "B".into(), ..b }, Server { id: "K".into(), ..k }];
        let built = build_auto(&servers, &Settings::default(), &local(false)).unwrap();
        assert_eq!(built.config["outbounds"][0]["type"], "urltest");
        assert_eq!(built.config["outbounds"][0]["outbounds"], json!(["s0", "s1"]));
        assert_eq!(built.members["s1"], "B");
        assert_eq!(built.config["route"]["final"], AUTO_TAG);
    }
}

/// `QUADRA_DUMP=<dir> cargo test dump_configs -- --ignored` writes sample configs to check with
/// `sing-box check -c` (the real binary is the judge of the format).
#[cfg(test)]
mod dump {
    use super::*;

    #[test]
    #[ignore]
    fn dump_configs() {
        let Some(dir) = std::env::var_os("QUADRA_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        let rules = dir.join("rules");
        let links = [
            ("reality", "vless://11111111-2222-3333-4444-555555555555@198.51.100.7:443?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=www.site.com&fp=firefox&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=ab12#x"),
            ("ws", "vless://11111111-2222-3333-4444-555555555555@h.example:443?type=ws&security=tls&path=%2Fws%3Fed%3D2048&host=cdn.example&fp=chrome#w"),
            ("grpc", "vless://11111111-2222-3333-4444-555555555555@198.51.100.1:2053?type=grpc&serviceName=svc&security=reality&sni=dzen.ru&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=0d#g"),
            ("trojan", "trojan://p@198.51.100.2:443?security=tls&sni=b&type=httpupgrade&path=%2Fu#b"),
            ("ss", "ss://Y2hhY2hhMjAtaWV0Zi1wb2x5MTMwNTpwYXNz@1.2.3.4:8388#n"),
            ("vmess", "vmess://eyJhZGQiOiIxLjIuMy40IiwicG9ydCI6IjQ0MyIsImlkIjoiMTExMTExMTEtMjIyMi0zMzMzLTQ0NDQtNTU1NTU1NTU1NTU1IiwibmV0Ijoid3MiLCJ0bHMiOiJ0bHMiLCJwYXRoIjoiL3YiLCJob3N0IjoiYS5iIiwicHMiOiJ2In0="),
            ("xhttp", "vless://11111111-2222-3333-4444-555555555555@198.51.100.9:443?type=xhttp&security=reality&sni=www.nvidia.com&pbk=jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0&sid=01&path=%2Fp&mode=auto#r"),
        ];
        let mut st = Settings { routing: "ru".into(), bypass_apps: vec!["Steam.exe".into()], bypass_domains: vec!["example.org".into()], ..Default::default() };
        st.rules.push(crate::model::Rule { kind: "domain".into(), value: "example.net".into(), action: "block".into() });
        st.dns = "1.1.1.1, https://dns.google/dns-query, tls://9.9.9.9".into();
        let mut all = vec![];
        for (name, uri) in links {
            let srv = crate::links::parse(uri).unwrap();
            for tun in [false, true] {
                let l = Local { tun, clash_port: 9090, probe_port: 0, secret: "s", rules_dir: &rules };
                let b = build(&srv, &st, &l).unwrap();
                std::fs::write(dir.join(format!("{name}-{}.json", if tun { "tun" } else { "ports" })), b.config.to_string()).unwrap();
            }
            all.push(Server { id: name.into(), ..srv });
        }
        let l = Local { tun: false, clash_port: 9090, probe_port: 0, secret: "s", rules_dir: &rules };
        std::fs::write(dir.join("auto-ports.json"), build_auto(&all, &st, &l).unwrap().config.to_string()).unwrap();
        let full_cfg = json!({
            "dns": {"servers": ["1.1.1.1", {"address": "8.8.8.8"}], "queryStrategy": "UseIPv4"},
            "inbounds": [{"tag": "socks-in", "port": 1, "protocol": "socks"}],
            "outbounds": [
                {"tag": "eu-1", "protocol": "trojan", "settings": {"servers": [{"address": "198.51.100.1", "port": 443, "password": "p"}]}, "streamSettings": {"security": "tls", "sockopt": {"dialerProxy": "frag"}}},
                {"tag": "eu-2", "protocol": "vless", "settings": {"vnext": [{"address": "198.51.100.2", "port": 443, "users": [{"id": "11111111-2222-3333-4444-555555555555", "encryption": "none"}]}]}, "streamSettings": {"network": "ws", "security": "tls", "wsSettings": {"path": "/x"}}},
                {"tag": "direct", "protocol": "freedom"}, {"tag": "frag", "protocol": "freedom", "settings": {"fragment": {"packets": "tlshello"}}},
                {"tag": "blocked", "protocol": "blackhole"}],
            "routing": {"domainStrategy": "IPIfNonMatch", "balancers": [{"tag": "lb", "selector": ["eu-"], "strategy": {"type": "leastPing"}}],
                "rules": [
                    {"type": "field", "domain": ["geosite:category-ru", "full:a.ru", "keyword:x", "regexp:^y"], "ip": ["geoip:ru"], "outboundTag": "direct"},
                    {"type": "field", "ip": ["geoip:private", "10.1.0.0/16"], "outboundTag": "direct"},
                    {"type": "field", "protocol": ["bittorrent"], "outboundTag": "direct"},
                    {"type": "field", "inboundTag": ["socks-in"], "network": "tcp,udp", "balancerTag": "lb"},
                    {"type": "field", "port": "25,465-587", "outboundTag": "blocked"}]}});
        let srv = Server { config: Some(full_cfg), ..Default::default() };
        for tun in [false, true] {
            let l = Local { tun, clash_port: 9090, probe_port: 0, secret: "s", rules_dir: &rules };
            std::fs::write(dir.join(format!("full-{}.json", if tun { "tun" } else { "ports" })), build(&srv, &st, &l).unwrap().config.to_string()).unwrap();
        }
    }
}
