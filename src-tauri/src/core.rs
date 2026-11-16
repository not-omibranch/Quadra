//! Running the core: pick the engine (sing-box, or Xray for what sing-box cannot do), start it,
//! confirm the tunnel really carries traffic, watch it, stop it.

use crate::model::{Home, Server, Settings, Status};
use crate::{config, emit_status, singbox, sys, Ctx};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Where the running core reports its traffic.
#[derive(Clone)]
enum Stats {
    /// Xray's `metrics` listener (`/debug/vars`).
    Xray { metrics: u16 },
    /// sing-box's clash API; `members` maps the auto group's outbounds to server ids.
    SingBox { clash: u16, secret: String, members: HashMap<String, String> },
}

pub struct Running {
    child: tokio::process::Child,
    stats: Stats,
    /// Started through the privileged wrapper (Unix, all-traffic mode): we cannot kill it,
    /// only ask it to stop.
    wrapped: bool,
    /// Whatever else has to be undone when the connection ends (Android: the system VPN).
    stopper: Option<Box<dyn FnOnce() + Send + Sync>>,
}

const PROXY_BACKUP: &str = "proxy_backup.json";
/// The core's output of the current and the previous connection, for looking at afterwards.
const LOG_FILE: &str = "core.log";

/// sing-box runs at the info level so that its "started" line arrives; its info lines (one per
/// connection) are dropped unless the user asked for a verbose log.
fn noise(ctx: &Ctx, line: &str) -> bool {
    line.starts_with("INFO") && !line.contains("sing-box started") && !ctx.verbose_core.load(Ordering::Relaxed)
}

fn log(app: &AppHandle, ctx: &Ctx, line: String) {
    // the core says so itself once every inbound is listening
    if (line.contains("core: Xray") && line.ends_with("started")) || line.contains("sing-box started") {
        ctx.core_ready.store(true, Ordering::SeqCst);
    }
    if noise(ctx, &line) {
        return;
    }
    let mut logs = ctx.logs.lock().unwrap();
    if logs.len() >= 1500 {
        logs.pop_front();
    }
    logs.push_back(line.clone());
    drop(logs);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(ctx.dir.join(LOG_FILE)) {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
    let _ = app.emit("log", line);
}

fn set_status(app: &AppHandle, ctx: &Ctx, f: impl FnOnce(&mut Status)) {
    f(&mut ctx.status.lock().unwrap());
    emit_status(app, ctx);
}

/// Puts back whatever system proxy was set before us. Safe to call when nothing was changed.
pub fn restore_proxy(ctx: &Ctx) {
    let path = ctx.dir.join(PROXY_BACKUP);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(backup) = serde_json::from_str::<sys::ProxyBackup>(&text) {
            let _ = sys::restore_system_proxy(&backup);
        }
        let _ = std::fs::remove_file(path);
    }
}

async fn stop(ctx: &Ctx) {
    if let Some(mut running) = ctx.core.lock().await.take() {
        if let Some(undo) = running.stopper.take() {
            undo();
        }
        if running.wrapped {
            sys::release_tun(&ctx.dir.join("run"));
            let _ = tokio::time::timeout(Duration::from_secs(3), running.child.wait()).await;
        }
        let _ = running.child.kill().await;
    }
    restore_proxy(ctx);
}

pub async fn disconnect(app: &AppHandle, ctx: &Arc<Ctx>, error: Option<String>) {
    ctx.gen.fetch_add(1, Ordering::SeqCst);
    stop(ctx).await;
    set_status(app, ctx, |s| {
        let server = s.server.take();
        *s = Status { state: "off".into(), error, ..Default::default() };
        s.server = server;
    });
}

async fn port_free(port: u16) -> bool {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.is_ok()
}

async fn free_port() -> u16 {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await {
        Ok(l) => l.local_addr().map(|a| a.port()).unwrap_or(20811),
        Err(_) => 20811,
    }
}

fn through(socks: u16, secs: u64) -> Option<reqwest::Client> {
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{socks}")).ok()?;
    reqwest::Client::builder().proxy(proxy).timeout(Duration::from_secs(secs)).build().ok()
}

/// A real request through the tunnel. The connection counts as up only when this succeeds.
async fn probe(socks: u16) -> Option<u32> {
    // A new connection to a server sometimes stalls for seconds (a lost first packet, a busy
    // server) while the next one goes through at once. So the question is asked several times,
    // a little apart, and the first good answer settles it: one unlucky connection must not
    // make the whole app look slow.
    const URLS: [&str; 2] = ["http://cp.cloudflare.com/generate_204", "http://www.gstatic.com/generate_204"];
    const STARTS_MS: [u64; 4] = [0, 500, 1300, 2600];
    let client = through(socks, 5)?;
    let began = Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<bool>(STARTS_MS.len() * URLS.len());
    let mut tasks = vec![];
    for (i, delay) in STARTS_MS.into_iter().enumerate() {
        let (client, tx) = (client.clone(), tx.clone());
        let url = URLS[i % URLS.len()];
        tasks.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            let good = client.get(url).send().await.map(|r| r.status().as_u16() < 400).unwrap_or(false);
            let _ = tx.send(good).await;
        }));
    }
    drop(tx);
    let mut answer = None;
    while let Some(good) = rx.recv().await {
        if good {
            answer = Some(began.elapsed().as_millis() as u32);
            break;
        }
    }
    for t in tasks {
        t.abort();
    }
    answer
}

fn is_ip(host: &str) -> bool {
    host.trim_matches(|c| c == '[' || c == ']').parse::<std::net::IpAddr>().is_ok()
}

/// Every server address in a config that is a name, not a number.
fn server_names(cfg: &Value) -> Vec<String> {
    let mut names = vec![];
    for o in cfg["outbounds"].as_array().into_iter().flatten() {
        let st = &o["settings"];
        let entries = st["vnext"].as_array().into_iter().flatten().chain(st["servers"].as_array().into_iter().flatten()).chain(std::iter::once(st));
        for e in entries {
            if let Some(a) = e["address"].as_str().filter(|a| !a.is_empty() && !is_ip(a)) {
                if !names.iter().any(|n| n == a) {
                    names.push(a.to_string());
                }
            }
        }
    }
    names
}

/// Writes the resolved addresses into the config. The name the server expects in the TLS
/// handshake and in the Host header stays the name: where the config relied on the address
/// for that, the name is now spelled out.
fn pin_addresses(cfg: &mut Value, resolved: &HashMap<String, std::net::IpAddr>) {
    let Some(outbounds) = cfg["outbounds"].as_array_mut() else { return };
    for o in outbounds {
        let mut name: Option<String> = None;
        let st = &mut o["settings"];
        let mut pin = |e: &mut Value| {
            let Some(host) = e["address"].as_str().map(String::from) else { return };
            if let Some(ip) = resolved.get(&host) {
                e["address"] = json!(ip.to_string());
                name.get_or_insert(host);
            }
        };
        for key in ["vnext", "servers"] {
            for e in st[key].as_array_mut().into_iter().flatten() {
                pin(e);
            }
        }
        if st.is_object() {
            pin(st);
        }
        let Some(name) = name else { continue };
        let stream = &mut o["streamSettings"];
        if !stream.is_object() {
            continue;
        }
        let empty = |v: &Value| v.as_str().unwrap_or("").is_empty();
        if stream["security"] == "tls" {
            if !stream["tlsSettings"].is_object() {
                stream["tlsSettings"] = json!({});
            }
            if empty(&stream["tlsSettings"]["serverName"]) {
                stream["tlsSettings"]["serverName"] = json!(name);
            }
        }
        for key in ["wsSettings", "httpupgradeSettings", "xhttpSettings"] {
            let s = &mut stream[key];
            if s.is_object() && empty(&s["host"]) && empty(&s["headers"]["Host"]) {
                s["host"] = json!(name);
            }
        }
        let grpc = &mut stream["grpcSettings"];
        if grpc.is_object() && empty(&grpc["authority"]) {
            grpc["authority"] = json!(name);
        }
    }
}

/// Looks the servers' names up now, before anything is started, and gives the core numbers.
/// Otherwise the core asks the system while the tunnel is coming up, and in the all-traffic
/// mode that question is sent into the very tunnel that is waiting for its answer.
/// Answers are remembered for ten minutes, so a reconnect does not ask again.
async fn resolve_servers(ctx: &Ctx, cfg: &mut Value) -> (usize, usize) {
    let names = server_names(cfg);
    if names.is_empty() {
        return (0, 0);
    }
    let mut resolved: HashMap<String, std::net::IpAddr> = HashMap::new();
    let mut missing = vec![];
    {
        let cache = ctx.resolved.lock().unwrap();
        for n in &names {
            match cache.get(n) {
                Some((ip, at)) if at.elapsed() < Duration::from_secs(600) => {
                    resolved.insert(n.clone(), *ip);
                }
                _ => missing.push(n.clone()),
            }
        }
    }
    let lookups = missing.into_iter().map(|name| {
        tokio::spawn(async move {
            let found = tokio::time::timeout(Duration::from_millis(2500), tokio::net::lookup_host((name.as_str(), 443))).await;
            let addrs: Vec<std::net::SocketAddr> = found.ok().and_then(|r| r.ok()).map(|a| a.collect()).unwrap_or_default();
            let ip = addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).map(|a| a.ip());
            (name, ip)
        })
    });
    for task in lookups.collect::<Vec<_>>() {
        if let Ok((name, Some(ip))) = task.await {
            ctx.resolved.lock().unwrap().insert(name.clone(), (ip, Instant::now()));
            resolved.insert(name, ip);
        }
    }
    pin_addresses(cfg, &resolved);
    (resolved.len(), names.len())
}

async fn where_am_i(client: &reqwest::Client) -> Option<Home> {
    for url in ["https://ipwho.is/", "https://ipapi.co/json/"] {
        let Ok(resp) = client.get(url).send().await else { continue };
        let Ok(text) = resp.text().await else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if let (Some(lat), Some(lon)) = (v["latitude"].as_f64(), v["longitude"].as_f64()) {
            return Some(Home {
                lat, lon,
                cc: v["country_code"].as_str().unwrap_or("").to_string(),
                ip: v["ip"].as_str().unwrap_or("").to_string(),
                manual: false,
            });
        }
    }
    None
}

/// The user's own location by IP, asked directly (never through a tunnel).
pub async fn locate_home() -> Option<Home> {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(8)).build().ok()?;
    where_am_i(&client).await
}

fn traffic_totals(vars: &Value) -> (u64, u64) {
    let (mut up, mut down) = (0, 0);
    if let Some(inbound) = vars["stats"]["inbound"].as_object() {
        for v in inbound.values() {
            up += v["uplink"].as_u64().unwrap_or(0);
            down += v["downlink"].as_u64().unwrap_or(0);
        }
    }
    (up, down)
}

/// Up and down totals the core has counted so far.
async fn totals(client: &reqwest::Client, stats: &Stats) -> Option<(u64, u64)> {
    match stats {
        Stats::Xray { metrics } => {
            let resp = client.get(format!("http://127.0.0.1:{metrics}/debug/vars")).send().await.ok()?;
            Some(traffic_totals(&resp.json_value().await.ok()?))
        }
        Stats::SingBox { clash, secret, .. } => {
            let resp = client.get(format!("http://127.0.0.1:{clash}/connections")).bearer_auth(secret).send().await.ok()?;
            let v = resp.json_value().await.ok()?;
            Some((v["uploadTotal"].as_u64().unwrap_or(0), v["downloadTotal"].as_u64().unwrap_or(0)))
        }
    }
}

/// The server sing-box's auto group is using right now.
async fn auto_pick(client: &reqwest::Client, clash: u16, secret: &str, members: &HashMap<String, String>) -> Option<String> {
    let resp = client.get(format!("http://127.0.0.1:{clash}/proxies/{}", singbox::AUTO_TAG)).bearer_auth(secret).send().await.ok()?;
    let v = resp.json_value().await.ok()?;
    members.get(v["now"].as_str()?).cloned()
}

/// Runs for as long as this connection is the current one: reports speed, notices a dead core,
/// follows the auto group's choice.
async fn watch(app: AppHandle, ctx: Arc<Ctx>, gen: u64) {
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().ok();
    let mut last: Option<(u64, u64, Instant)> = None;
    loop {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        if ctx.gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let stats = {
            let mut core = ctx.core.lock().await;
            match core.as_mut() {
                Some(r) => match r.child.try_wait() {
                    Ok(None) => r.stats.clone(),
                    _ => {
                        drop(core);
                        disconnect(&app, &ctx, Some("ядро неожиданно остановилось, подробности в журнале".into())).await;
                        return;
                    }
                },
                None => return,
            }
        };
        let Some(client) = &client else { continue };
        if let Stats::SingBox { clash, secret, members } = &stats {
            if !members.is_empty() {
                if let Some(id) = auto_pick(client, *clash, secret, members).await {
                    let current = ctx.status.lock().unwrap().server.clone();
                    if ctx.gen.load(Ordering::SeqCst) == gen && current.as_deref() != Some(id.as_str()) {
                        log(&app, &ctx, format!("авто: теперь {}", server_name(&ctx, &id)));
                        set_status(&app, &ctx, |s| s.server = Some(id));
                    }
                }
            }
        }
        let Some((up, down)) = totals(client, &stats).await else { continue };
        let now = Instant::now();
        if let Some((pu, pd, pt)) = last {
            let dt = now.duration_since(pt).as_secs_f64().max(0.2);
            let _ = app.emit("traffic", json!({
                "up": (up.saturating_sub(pu) as f64 / dt) as u64,
                "down": (down.saturating_sub(pd) as f64 / dt) as u64,
                "tup": up, "tdown": down}));
        }
        last = Some((up, down, now));
    }
}

/// How often the background check runs while the server does not answer, and while it does.
const RECHECK_FAILING: Duration = Duration::from_secs(12);
const RECHECK_FINE: Duration = Duration::from_secs(60);

/// The background check of a connection made without waiting for the server (the default):
/// asks through the tunnel, marks the status "ok" or "fail", logs every change, and never
/// tears the connection down. A server that does not answer is the server's problem; the
/// tunnel and the system proxy stay as they are.
async fn keep_checking(app: AppHandle, ctx: Arc<Ctx>, gen: u64, socks: u16) {
    let mut last: Option<bool> = None;
    loop {
        if ctx.gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let asked = Instant::now();
        let ms = probe(socks).await;
        if ctx.gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let good = ms.is_some();
        match (last, ms) {
            (Some(true), Some(_)) => {}
            (Some(false), None) => {}
            (None, Some(ms)) => log(&app, &ctx, format!("проверка связи: сервер ответил за {ms} мс")),
            (Some(false), Some(ms)) => log(&app, &ctx, format!("проверка связи: сервер снова отвечает, {ms} мс")),
            (_, None) => log(&app, &ctx, format!(
                "проверка связи: за {} мс через сервер не прошёл ни один запрос; соединение оставлено, повтор через {} с",
                asked.elapsed().as_millis(), RECHECK_FAILING.as_secs())),
        }
        last = Some(good);
        set_status(&app, &ctx, |s| {
            if s.state == "on" && ctx.gen.load(Ordering::SeqCst) == gen {
                s.check = if good { "ok" } else { "fail" }.into();
                s.ms = ms;
            }
        });
        let pause = if good { RECHECK_FINE } else { RECHECK_FAILING };
        let until = Instant::now() + pause;
        while Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if ctx.gen.load(Ordering::SeqCst) != gen {
                return;
            }
        }
    }
}

fn server_name(ctx: &Ctx, id: &str) -> String {
    ctx.data.lock().unwrap().servers.iter().find(|s| s.id == id).map(|s| s.name.clone()).unwrap_or_else(|| id.to_string())
}

trait JsonValue {
    async fn json_value(self) -> Result<Value, ()>;
}

impl JsonValue for reqwest::Response {
    async fn json_value(self) -> Result<Value, ()> {
        let text = self.text().await.map_err(|_| ())?;
        serde_json::from_str(&text).map_err(|_| ())
    }
}

/// Why this connection has to be made by Xray rather than sing-box, if it has to.
fn xray_reason(ctx: &Ctx, server: &Server, settings: &Settings) -> Option<String> {
    if cfg!(target_os = "android") {
        return Some("sing-box для Android ещё не собран".into());
    }
    if !ctx.singbox.join(sys::SINGBOX_BIN).is_file() {
        return Some("sing-box не найден рядом с приложением".into());
    }
    if settings.core == "xray" {
        return Some("так выбрано в настройках".into());
    }
    if let Some(r) = singbox::needs_xray(server) {
        return Some(format!("sing-box не умеет: {r}"));
    }
    None
}

/// Servers for "Авто, самый быстрый": everything sing-box can carry, the ones with a known
/// ping first (fastest first), at most 30.
fn auto_candidates(ctx: &Ctx, settings: &Settings) -> Vec<Server> {
    let d = ctx.data.lock().unwrap();
    let pings = ctx.pings.lock().unwrap();
    let mut list: Vec<(i32, Server)> = d.servers.iter()
        .filter(|s| singbox::primary(s).is_some_and(|o| singbox::outbound(&o, "x").is_ok()))
        .map(|s| (pings.get(&s.id).copied().filter(|p| *p > 0).unwrap_or(i32::MAX), s.clone()))
        .collect();
    list.sort_by_key(|(p, _)| *p);
    list.into_iter().take(30).map(|(_, s)| s).collect()
}

/// The single server the UI would pick for "auto": the lowest TCP ping.
fn fastest(ctx: &Ctx) -> Option<String> {
    let d = ctx.data.lock().unwrap();
    let pings = ctx.pings.lock().unwrap();
    d.servers.iter().filter_map(|s| pings.get(&s.id).filter(|p| **p > 0).map(|p| (*p, s.id.clone()))).min().map(|(_, id)| id)
        .or_else(|| d.servers.first().map(|s| s.id.clone()))
}

fn random_secret() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    crate::subs::short_hash(&format!("{t}-{}-{:p}", std::process::id(), &t))
}

/// A server whose addresses have been looked up and written in as numbers.
async fn pinned(ctx: &Ctx, server: &Server) -> (Server, usize, usize) {
    let mut out = server.clone();
    let (found, asked) = match (&server.config, &server.outbound) {
        (Some(c), _) => {
            let mut c = c.clone();
            let r = resolve_servers(ctx, &mut c).await;
            out.config = Some(c);
            r
        }
        (None, Some(o)) => {
            let mut wrap = json!({"outbounds": [o]});
            let r = resolve_servers(ctx, &mut wrap).await;
            out.outbound = Some(wrap["outbounds"][0].clone());
            r
        }
        _ => (0, 0),
    };
    (out, found, asked)
}

/// What the connection is about to start.
struct Plan {
    cfg: Value,
    bin: std::path::PathBuf,
    args: Vec<String>,
    stats: Stats,
    engine: &'static str,
    notes: Vec<String>,
    found: usize,
    asked: usize,
}

pub async fn connect(app: AppHandle, ctx: Arc<Ctx>, id: String) -> Result<(), String> {
    let began = Instant::now();
    let gen = ctx.gen.fetch_add(1, Ordering::SeqCst) + 1;
    let launch = ctx.launch.lock().await;
    if ctx.gen.load(Ordering::SeqCst) != gen {
        return Ok(()); // a newer request is already waiting its turn
    }
    stop(&ctx).await;
    let settings = ctx.data.lock().unwrap().settings.clone();
    // "auto": sing-box's urltest group over many servers; when that is not possible, the one
    // server with the lowest ping, as before
    let mut group: Vec<Server> = vec![];
    let id = if id == "auto" {
        let candidates = auto_candidates(&ctx, &settings);
        let usable = !cfg!(target_os = "android") && settings.core != "xray" && ctx.singbox.join(sys::SINGBOX_BIN).is_file();
        if usable && !candidates.is_empty() {
            let first = candidates[0].id.clone();
            group = candidates;
            first
        } else {
            fastest(&ctx).ok_or("нет серверов")?
        }
    } else {
        id
    };
    let server = {
        let d = ctx.data.lock().unwrap();
        d.servers.iter().find(|s| s.id == id).cloned().ok_or("сервер не найден")?
    };
    // a phone has one mode: everything through the system VPN
    let tun = settings.mode == "tun" || cfg!(target_os = "android");
    // ...and there the tunnel device is ours, not the core's (see android.rs)
    let core_tun = tun && !cfg!(target_os = "android");
    set_status(&app, &ctx, |s| *s = Status { state: "connecting".into(), server: Some(id.clone()), tun, ..Default::default() });
    ctx.verbose_core.store(matches!(settings.log_level.as_str(), "info" | "debug"), Ordering::SeqCst);

    let fail = |msg: String| {
        let (app, ctx) = (app.clone(), ctx.clone());
        async move {
            if ctx.gen.load(Ordering::SeqCst) == gen {
                log(&app, &ctx, format!("не удалось за {} мс: {msg}", began.elapsed().as_millis()));
                disconnect(&app, &ctx, Some(msg.clone())).await;
            }
            Err::<(), String>(msg)
        }
    };

    if tun && sys::needs_relaunch_for_tun() {
        return fail("режим «весь трафик» требует прав администратора".into()).await;
    }
    if core_tun {
        if let Ok(Some(other)) = tokio::task::spawn_blocking(sys::other_tunnel).await {
            return fail(format!("уже работает другой VPN (адаптер {other}): два туннеля мешают друг другу. Выключи его или выбери режим «Прокси»")).await;
        }
    }
    for port in [settings.socks_port, settings.http_port] {
        let mut free = port_free(port).await;
        for _ in 0..20 {
            if free {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            free = port_free(port).await;
        }
        if !free {
            return fail(format!("порт {port} занят другой программой: смени его в настройках")).await;
        }
    }
    let run_dir = ctx.dir.join("run");
    let _ = std::fs::create_dir_all(&run_dir);
    let cfg_path = run_dir.join("config.json");
    let cfg_arg = cfg_path.to_string_lossy().to_string();
    let lookup = Instant::now();
    let reason = if group.is_empty() { xray_reason(&ctx, &server, &settings) } else { None };
    let plan = match &reason {
        Some(_) => {
            let metrics = free_port().await;
            let mut cfg = match config::build(&server, &settings, core_tun, metrics) {
                Ok(c) => c,
                Err(e) => return fail(e).await,
            };
            let (found, asked) = resolve_servers(&ctx, &mut cfg).await;
            Plan { cfg, bin: ctx.xray.join(sys::XRAY_BIN), args: vec!["run".into(), "-c".into(), cfg_arg.clone()],
                   stats: Stats::Xray { metrics }, engine: "Xray", notes: vec![], found, asked }
        }
        None => {
            let clash = free_port().await;
            let probe_port = free_port().await;
            let secret = random_secret();
            let rules = ctx.singbox.join("rules");
            let local = singbox::Local { tun: core_tun, clash_port: clash, probe_port, secret: &secret, rules_dir: &rules };
            let (built, found, asked) = if group.is_empty() {
                let (srv, found, asked) = pinned(&ctx, &server).await;
                (singbox::build(&srv, &settings, &local), found, asked)
            } else {
                let mut wrap = json!({"outbounds": group.iter().filter_map(singbox::primary).collect::<Vec<_>>()});
                let (found, asked) = resolve_servers(&ctx, &mut wrap).await;
                let mut pinned_group = vec![];
                let mut k = 0;
                for s in &group {
                    if singbox::primary(s).is_none() {
                        continue;
                    }
                    pinned_group.push(Server { outbound: Some(wrap["outbounds"][k].clone()), config: None, ..s.clone() });
                    k += 1;
                }
                (singbox::build_auto(&pinned_group, &settings, &local), found, asked)
            };
            let built = match built {
                Ok(b) => b,
                Err(e) => return fail(e).await,
            };
            Plan { cfg: built.config, bin: ctx.singbox.join(sys::SINGBOX_BIN),
                   args: vec!["run".into(), "--disable-color".into(), "-c".into(), cfg_arg.clone()],
                   stats: Stats::SingBox { clash, secret, members: built.members }, engine: "sing-box", notes: built.notes, found, asked }
        }
    };
    let lookup_ms = lookup.elapsed().as_millis();
    // the check asks through its own port when the core has one (sing-box), else the user's SOCKS port
    let check_port = plan.cfg["inbounds"].as_array().into_iter().flatten()
        .find(|i| i["tag"] == singbox::PROBE_TAG).and_then(|i| i["listen_port"].as_u64()).map(|p| p as u16)
        .unwrap_or(settings.socks_port);
    if let Err(e) = std::fs::write(&cfg_path, serde_json::to_vec(&plan.cfg).unwrap_or_default()) {
        return fail(format!("не удалось записать конфигурацию: {e}")).await;
    }

    ctx.core_ready.store(false, Ordering::SeqCst);
    let started = Instant::now();
    let wrapper = if core_tun && !sys::is_admin() { sys::tun_wrapper(&plan.bin, &plan.args, &ctx.assets, &run_dir) } else { None };
    let wrapped = wrapper.is_some();
    if core_tun && cfg!(unix) && !wrapped && !sys::is_admin() {
        return fail("для режима «весь трафик» нужен pkexec (пакет polkit): установи его или выбери другой режим".into()).await;
    }
    let mut cmd = wrapper.unwrap_or_else(|| {
        let mut c = tokio::process::Command::new(&plan.bin);
        c.args(&plan.args).current_dir(&run_dir).env("XRAY_LOCATION_ASSET", &ctx.assets);
        if plan.engine == "Xray" {
            c.current_dir(&ctx.assets);
        }
        c
    });
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    sys::prepare(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("не удалось запустить ядро {}: {e}", plan.engine)).await,
    };
    sys::adopt(&child);
    let pid = child.id();
    if wrapped {
        // the wrapper sends the core's output to a file; follow it
        let (app, ctx, path) = (app.clone(), ctx.clone(), run_dir.join("core.out"));
        tokio::spawn(async move {
            let mut seen = 0usize;
            while ctx.gen.load(Ordering::SeqCst) == gen {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let Some(end) = text.rfind('\n') else { continue };
                if end + 1 > seen {
                    for line in text[seen..=end].lines() {
                        log(&app, &ctx, line.to_string());
                    }
                    seen = end + 1;
                }
            }
        });
    }
    let log_path = ctx.dir.join(LOG_FILE);
    if std::fs::metadata(&log_path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::rename(&log_path, ctx.dir.join("core.old.log"));
    }
    let what = if group.is_empty() { server.name.clone() } else { format!("авто из {} серверов", group.len()) };
    let engine_note = match &reason {
        Some(r) => format!("ядро Xray: {r}"),
        None => "ядро sing-box".into(),
    };
    log(&app, &ctx, format!("--- подключение: {what} | режим {} | {} | {engine_note} ---", if tun { "tun" } else { settings.mode.as_str() },
        if sys::is_admin() { "администратор" } else { "обычные права" }));
    for n in &plan.notes {
        log(&app, &ctx, n.clone());
    }
    if let Some(out) = child.stdout.take() {
        let (app, ctx) = (app.clone(), ctx.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                log(&app, &ctx, l);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let (app, ctx) = (app.clone(), ctx.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                log(&app, &ctx, l);
            }
        });
    }

    let stats = plan.stats.clone();
    *ctx.core.lock().await = Some(Running { child, stats, wrapped, stopper: None });
    drop(launch);
    // a password prompt may stand between us and the core
    let patience = Duration::from_secs(if wrapped { 120 } else { 10 });

    loop {
        if ctx.gen.load(Ordering::SeqCst) != gen {
            return Ok(());
        }
        let exited = match ctx.core.lock().await.as_mut() {
            Some(r) => !matches!(r.child.try_wait(), Ok(None)),
            None => return Ok(()),
        };
        if exited {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let reason = ctx.logs.lock().unwrap().iter().rev()
                .find(|l| l.contains("Failed") || l.contains("failed") || l.contains("rror") || l.starts_with("FATAL") || l.starts_with("ERROR"))
                .cloned().unwrap_or_else(|| "подробности в журнале".into());
            return fail(format!("ядро не запустилось: {}", reason.chars().take(220).collect::<String>())).await;
        }
        // Ready when the core reports it. Probing the port instead costs half a second on
        // Windows, where a refused local connection takes that long to come back; it is kept
        // only as a fallback for a core that does not say "started".
        if ctx.core_ready.load(Ordering::SeqCst) {
            break;
        }
        if started.elapsed() > Duration::from_millis(1500) && TcpStream::connect((Ipv4Addr::LOCALHOST, settings.socks_port)).await.is_ok() {
            break;
        }
        if started.elapsed() > patience {
            return fail("ядро не запустилось вовремя".into()).await;
        }
        tokio::time::sleep(Duration::from_millis(12)).await;
    }

    let core_ms = started.elapsed().as_millis();
    let probe_began = Instant::now();
    // Strict (settings.verify_first): the connection counts only after a request through the
    // server has come back. Otherwise it is up as soon as the core runs, the way other clients
    // behave; whether the server answers is checked in the background and shown as a mark.
    let strict = settings.verify_first;
    let ms = if strict {
        let ms = probe(check_port).await;
        if ctx.gen.load(Ordering::SeqCst) != gen {
            return Ok(());
        }
        let Some(ms) = ms else {
            return fail(format!("сервер не отвечает: за {} мс через него не прошёл ни один запрос", probe_began.elapsed().as_millis())).await;
        };
        Some(ms)
    } else {
        None
    };
    let probe_ms = probe_began.elapsed().as_millis();

    #[cfg(target_os = "android")]
    {
        // the core answers; now put the system VPN in front of it
        match crate::android::start_tunnel(&app, &ctx, settings.socks_port, gen).await {
            Ok(undo) => match ctx.core.lock().await.as_mut() {
                Some(running) => running.stopper = Some(undo),
                None => {
                    undo();
                    return Ok(());
                }
            },
            Err(e) => return fail(e).await,
        }
    }
    if settings.mode == "proxy" && cfg!(not(target_os = "android")) {
        // domains the system itself sends past the proxy; in "only" mode the list means the opposite
        let past: &[String] = if settings.bypass_mode == "only" { &[] } else { &settings.bypass_domains };
        match sys::set_system_proxy(settings.http_port, settings.socks_port, past) {
            Ok(backup) => {
                let _ = std::fs::write(ctx.dir.join(PROXY_BACKUP), serde_json::to_vec(&backup).unwrap_or_default());
            }
            Err(e) => {
                return fail(format!("не удалось включить системный прокси: {e}")).await;
            }
        }
    }
    let (found, asked) = (plan.found, plan.asked);
    let check_note = match ms {
        Some(ms) => format!("проверка связи {ms} мс"),
        None => "связь через сервер проверяется в фоне".into(),
    };
    log(&app, &ctx, format!(
        "подключено за {} мс: адреса {found} из {asked} за {lookup_ms} мс, ядро {} {core_ms} мс, {check_note}",
        began.elapsed().as_millis(), plan.engine));
    *ctx.timing.lock().unwrap() = json!({"engine": plan.engine, "total_ms": began.elapsed().as_millis() as u64,
        "lookup_ms": lookup_ms as u64, "core_ms": core_ms as u64, "probe_ms": probe_ms as u64, "first_answer_ms": ms,
        "strict": strict, "pid": pid});
    set_status(&app, &ctx, |s| {
        s.state = "on".into();
        s.since = crate::now();
        s.ms = ms;
        s.check = if strict { "ok" } else { "checking" }.into();
    });

    tokio::spawn(watch(app.clone(), ctx.clone(), gen));
    if !strict {
        tokio::spawn(keep_checking(app.clone(), ctx.clone(), gen, check_port));
    }
    if settings.geo_lookup {
        let socks = settings.socks_port;
        tokio::spawn(async move {
            let Some(client) = through(socks, 8) else { return };
            if let Some(exit) = where_am_i(&client).await {
                if ctx.gen.load(Ordering::SeqCst) == gen {
                    set_status(&app, &ctx, |s| s.exit = serde_json::to_value(&exit).ok());
                }
            }
        });
    }
    Ok(())
}

async fn tcp_ping(host: String, port: u16) -> i32 {
    let Ok(Ok(mut addrs)) = tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host((host.as_str(), port))).await else { return -1 };
    let Some(addr) = addrs.next() else { return -1 };
    let mut best = -1i32;
    for _ in 0..2 {
        let t = Instant::now();
        if let Ok(Ok(_)) = tokio::time::timeout(Duration::from_millis(2500), TcpStream::connect(addr)).await {
            let ms = t.elapsed().as_millis() as i32;
            best = if best < 0 { ms } else { best.min(ms) };
        } else if best < 0 {
            break;
        }
    }
    best
}

/// Time to open a TCP connection to each server, measured in parallel.
pub async fn ping(app: AppHandle, ctx: Arc<Ctx>, ids: Vec<String>) {
    let targets: Vec<(String, String, u16)> = {
        let d = ctx.data.lock().unwrap();
        d.servers.iter().filter(|s| ids.is_empty() || ids.contains(&s.id)).filter(|s| !s.address.is_empty())
            .map(|s| (s.id.clone(), s.address.clone(), s.port)).collect()
    };
    let limit = Arc::new(tokio::sync::Semaphore::new(24));
    let mut tasks = vec![];
    for (id, host, port) in targets {
        let (app, ctx, limit) = (app.clone(), ctx.clone(), limit.clone());
        tasks.push(tokio::spawn(async move {
            let _permit = limit.acquire().await;
            let ms = tcp_ping(host, port).await;
            ctx.pings.lock().unwrap().insert(id.clone(), ms);
            let _ = app.emit("ping", json!({"id": id, "ms": ms}));
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_become_numbers_and_names_stay_where_the_server_needs_them() {
        let mut cfg = json!({"outbounds": [
            {"protocol": "vless", "settings": {"vnext": [{"address": "de.example.net", "port": 443}]},
             "streamSettings": {"network": "tcp", "security": "reality", "realitySettings": {"serverName": "www.site.com"}}},
            {"protocol": "vless", "settings": {"vnext": [{"address": "cdn.example.net", "port": 443}]},
             "streamSettings": {"network": "ws", "security": "tls", "wsSettings": {"path": "/x"}}},
            {"protocol": "trojan", "settings": {"servers": [{"address": "203.0.113.9", "port": 443}]}},
            {"protocol": "freedom"}]});
        assert_eq!(server_names(&cfg), vec!["de.example.net", "cdn.example.net"]);
        let resolved: HashMap<String, std::net::IpAddr> = [("de.example.net", "198.51.100.1"), ("cdn.example.net", "198.51.100.2")]
            .into_iter().map(|(k, v)| (k.to_string(), v.parse().unwrap())).collect();
        pin_addresses(&mut cfg, &resolved);
        let o = &cfg["outbounds"];
        assert_eq!(o[0]["settings"]["vnext"][0]["address"], "198.51.100.1");
        assert_eq!(o[0]["streamSettings"]["realitySettings"]["serverName"], "www.site.com");
        assert_eq!(o[1]["settings"]["vnext"][0]["address"], "198.51.100.2");
        assert_eq!(o[1]["streamSettings"]["tlsSettings"]["serverName"], "cdn.example.net");
        assert_eq!(o[1]["streamSettings"]["wsSettings"]["host"], "cdn.example.net");
        assert_eq!(o[2]["settings"]["servers"][0]["address"], "203.0.113.9");
        assert!(server_names(&cfg).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn asking_the_system_for_tunnels_is_instant_and_does_not_crash() {
        let t = Instant::now();
        let found = sys::other_tunnel();
        println!("other tunnel: {found:?} in {:?}", t.elapsed());
        assert!(t.elapsed() < Duration::from_millis(200));
    }
}
