//! The whole backend. `main.rs` only calls `run()`; on Android the same function is the entry
//! point of the native library.

#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "android", test))]
mod sysdns;
mod config;
mod core;
mod geoip;
mod links;
mod model;
mod names;
mod singbox;
mod subs;
mod sys;

use model::{Data, Home, Settings, Status};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

pub struct Ctx {
    pub dir: PathBuf,
    /// Where the Xray binary is.
    pub xray: PathBuf,
    /// Where sing-box's rule-sets are (`rules/`); on a desktop the binary is there too.
    pub singbox: PathBuf,
    /// The sing-box binary itself (on Android it is a native library of the package).
    pub singbox_bin: PathBuf,
    /// Where geoip.dat and geosite.dat are (next to the binary, except on Android).
    pub assets: PathBuf,
    pub data: Mutex<Data>,
    pub pings: Mutex<HashMap<String, i32>>,
    pub status: Mutex<Status>,
    pub logs: Mutex<VecDeque<String>>,
    pub core: tokio::sync::Mutex<Option<core::Running>>,
    /// Held while a core is being stopped and the next one started, so two connects never overlap.
    pub launch: tokio::sync::Mutex<()>,
    pub gen: AtomicU64,
    /// The core that is starting has reported that it is up.
    pub core_ready: AtomicBool,
    /// Pass sing-box's per-connection lines on to the log (the user asked for a verbose level).
    pub verbose_core: AtomicBool,
    /// How long the last successful connection took, by stage (for the debug bench).
    pub timing: Mutex<Value>,
    /// Server names already looked up, and when (see core::resolve_servers).
    pub resolved: Mutex<HashMap<String, (std::net::IpAddr, std::time::Instant)>>,
    pub hidden: bool,
    pub elevated: bool,
    /// Under a tiling window manager: no splash window, no window buttons, no hiding to a tray.
    pub tiling: bool,
    /// The startup cube's window is still up.
    pub splash: AtomicBool,
    /// The main window has finished loading.
    pub main_ready: AtomicBool,
}

impl Ctx {
    pub fn save(&self) {
        let bytes = serde_json::to_vec(&*self.data.lock().unwrap()).unwrap_or_default();
        let tmp = self.dir.join("state.json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, self.dir.join("state.json"));
        }
    }
}

type Shared<'a> = State<'a, Arc<Ctx>>;

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn emit_status(app: &AppHandle, ctx: &Ctx) {
    let _ = app.emit("status", ctx.status.lock().unwrap().clone());
}

fn snapshot(ctx: &Ctx) -> Value {
    let d = ctx.data.lock().unwrap();
    let pings = ctx.pings.lock().unwrap();
    json!({
        "settings": d.settings,
        "subs": d.subs,
        "servers": d.servers.iter().map(|s| s.view(pings.get(&s.id).copied())).collect::<Vec<_>>(),
        "status": *ctx.status.lock().unwrap(),
        "hwid": d.hwid,
        "admin": sys::is_admin(),
        "platform": sys::PLATFORM,
        "tiling": ctx.tiling,
        "relaunchForTun": sys::needs_relaunch_for_tun(),
        "elevated": ctx.elevated,
        "version": env!("CARGO_PKG_VERSION"),
        "userAgent": subs::default_user_agent(),
    })
}

fn changed(app: &AppHandle, ctx: &Ctx) {
    ctx.save();
    let _ = app.emit("data", snapshot(ctx));
}

/// Servers named without a flag get their country from where their address points.
async fn fill_countries(ctx: &Arc<Ctx>, servers: &mut [model::Server]) {
    let dat = ctx.assets.join("geoip.dat");
    for s in servers.iter_mut().filter(|s| s.cc.is_empty() && !s.address.is_empty()) {
        let ip = match s.address.parse::<IpAddr>() {
            Ok(ip) => Some(ip),
            Err(_) => tokio::time::timeout(std::time::Duration::from_secs(2), tokio::net::lookup_host((s.address.as_str(), s.port)))
                .await.ok().and_then(|r| r.ok()).and_then(|mut a| a.find(|a| a.is_ipv4())).map(|a| a.ip()),
        };
        if let Some(IpAddr::V4(v4)) = ip {
            let dat = dat.clone();
            if let Ok(Some(cc)) = tokio::task::spawn_blocking(move || geoip::country(&dat, v4)).await {
                s.cc = cc;
            }
        }
    }
}

async fn refresh(app: &AppHandle, ctx: &Arc<Ctx>, id: &str, url: &str) -> Result<Value, String> {
    let (hwid, send, ua) = {
        let d = ctx.data.lock().unwrap();
        (d.hwid.clone(), d.settings.send_hwid, d.settings.user_agent.clone())
    };
    match subs::fetch(url, id, &hwid, send, &ua).await {
        Ok(mut f) => {
            fill_countries(ctx, &mut f.servers).await;
            let count = f.servers.len();
            {
                let mut d = ctx.data.lock().unwrap();
                d.servers.retain(|s| s.sub.as_deref() != Some(id));
                d.servers.extend(f.servers);
                match d.subs.iter_mut().find(|s| s.id == id) {
                    Some(s) => *s = f.sub,
                    None => d.subs.push(f.sub),
                }
            }
            changed(app, ctx);
            Ok(json!({"servers": count, "skipped": f.skipped}))
        }
        Err(e) => {
            let known = {
                let mut d = ctx.data.lock().unwrap();
                match d.subs.iter_mut().find(|s| s.id == id) {
                    Some(s) => { s.error = e.clone(); true }
                    None => false,
                }
            };
            if known {
                changed(app, ctx);
            }
            Err(e)
        }
    }
}

#[tauri::command]
fn get_state(ctx: Shared) -> Value {
    snapshot(&ctx)
}

/// Takes the startup cube away and makes sure the main window is up.
fn reveal(app: &AppHandle, ctx: &Ctx) {
    if ctx.splash.swap(false, Ordering::SeqCst) {
        if let Some(w) = app.get_webview_window("splash") {
            let _ = w.close();
        }
        // after the close, or the window that was active before us takes the front again
        show_main(app);
    }
}

/// The interface is loaded. With a splash on screen the cube first finishes its turn; started
/// hidden at login, nothing is shown at all.
#[tauri::command]
fn ready(app: AppHandle, ctx: Shared) {
    if cfg!(debug_assertions) && std::env::var_os("QUADRA_SPLASH_HOLD").is_some() {
        return; // development aid: keep the startup cube on screen to look at it
    }
    ctx.main_ready.store(true, Ordering::SeqCst);
    if ctx.hidden {
        return;
    }
    if !ctx.splash.load(Ordering::SeqCst) {
        show_main(&app);
        return;
    }
    let _ = app.emit_to("splash", "finish", ());
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn(async move {
        // never leave the user looking at a cube: if the splash does not answer, go on without it
        tokio::time::sleep(std::time::Duration::from_millis(4000)).await;
        reveal(&app, &ctx);
    });
}

/// From the splash window: "loaded" shows it and tells whether the main window is already waiting,
/// "leaving" brings the main window up under the fading cube, "done" closes the splash.
#[tauri::command]
fn splash(app: AppHandle, ctx: Shared, stage: String) -> bool {
    match stage.as_str() {
        // The window is created hidden: until the page has painted, a transparent window is
        // just a black square. It comes up with the cube already drawn.
        "loaded" => {
            if let Some(w) = app.get_webview_window("splash") {
                let _ = w.show();
            }
        }
        "leaving" => show_main(&app),
        "done" => reveal(&app, &ctx),
        _ => {}
    }
    ctx.main_ready.load(Ordering::SeqCst)
}

#[tauri::command]
fn save_settings(app: AppHandle, ctx: Shared, settings: Settings) {
    let autostart = {
        let mut d = ctx.data.lock().unwrap();
        let before = d.settings.autostart;
        d.settings = settings;
        (before != d.settings.autostart).then_some(d.settings.autostart)
    };
    #[cfg(desktop)]
    if let Some(on) = autostart {
        let launcher = app.autolaunch();
        let _ = if on { launcher.enable() } else { launcher.disable() };
    }
    let _ = (&app, autostart);
    ctx.save();
}

#[tauri::command]
async fn add_subscription(app: AppHandle, ctx: Shared<'_>, url: String) -> Result<Value, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("это не ссылка на подписку".into());
    }
    let id = subs::short_hash(&url);
    refresh(&app, ctx.inner(), &id, &url).await
}

#[tauri::command]
async fn refresh_subscription(app: AppHandle, ctx: Shared<'_>, id: String) -> Result<Value, String> {
    let url = ctx.data.lock().unwrap().subs.iter().find(|s| s.id == id).map(|s| s.url.clone()).ok_or("подписка не найдена")?;
    refresh(&app, ctx.inner(), &id, &url).await
}

#[tauri::command]
async fn delete_subscription(app: AppHandle, ctx: Shared<'_>, id: String) -> Result<(), String> {
    let active = ctx.status.lock().unwrap().server.clone();
    let hit = {
        let mut d = ctx.data.lock().unwrap();
        let hit = d.servers.iter().any(|s| s.sub.as_deref() == Some(&id) && Some(&s.id) == active.as_ref());
        d.servers.retain(|s| s.sub.as_deref() != Some(id.as_str()));
        d.subs.retain(|s| s.id != id);
        let selected = d.settings.selected.clone();
        if selected.is_some_and(|sel| !d.servers.iter().any(|s| s.id == sel)) {
            d.settings.selected = None;
        }
        hit
    };
    if hit {
        core::disconnect(&app, ctx.inner(), None).await;
    }
    changed(&app, &ctx);
    Ok(())
}

#[tauri::command]
async fn delete_server(app: AppHandle, ctx: Shared<'_>, id: String) -> Result<(), String> {
    if ctx.status.lock().unwrap().server.as_deref() == Some(id.as_str()) {
        core::disconnect(&app, ctx.inner(), None).await;
    }
    {
        let mut d = ctx.data.lock().unwrap();
        d.servers.retain(|s| s.id != id);
        if d.settings.selected.as_deref() == Some(id.as_str()) {
            d.settings.selected = None;
        }
    }
    changed(&app, &ctx);
    Ok(())
}

/// Anything pasted or dropped: subscription addresses, share links, a base64 list, an Xray config.
#[tauri::command]
async fn import_text(app: AppHandle, ctx: Shared<'_>, text: String) -> Result<Value, String> {
    let text = text.trim();
    let is_url = |l: &str| l.starts_with("https://") || l.starts_with("http://");
    let urls: Vec<String> = text.lines().map(str::trim).filter(|l| is_url(l)).map(String::from).collect();
    let rest = if text.starts_with('[') || text.starts_with('{') { text.to_string() } else {
        text.lines().map(str::trim).filter(|l| !is_url(l)).collect::<Vec<_>>().join("\n")
    };
    let (mut added_subs, mut errors) = (0, vec![]);
    for url in urls {
        match refresh(&app, ctx.inner(), &subs::short_hash(&url), &url).await {
            Ok(_) => added_subs += 1,
            Err(e) => errors.push(e),
        }
    }
    let (mut servers, skipped) = subs::parse_body(&rest);
    subs::assign_ids(&mut servers, None);
    fill_countries(ctx.inner(), &mut servers).await;
    let added = {
        let mut d = ctx.data.lock().unwrap();
        let fresh: Vec<_> = servers.into_iter().filter(|n| !d.servers.iter().any(|s| s.id == n.id)).collect();
        let n = fresh.len();
        d.servers.extend(fresh);
        n
    };
    if added > 0 {
        changed(&app, &ctx);
    }
    if added == 0 && added_subs == 0 {
        return Err(errors.into_iter().next().unwrap_or_else(|| {
            if skipped > 0 { "этот тип ссылок не поддерживается".into() } else { "здесь нет ни ссылок на серверы, ни подписок".into() }
        }));
    }
    Ok(json!({"servers": added, "subs": added_subs, "skipped": skipped, "errors": errors}))
}

#[tauri::command]
async fn connect(app: AppHandle, ctx: Shared<'_>, id: String) -> Result<(), String> {
    core::connect(app, ctx.inner().clone(), id).await
}

#[tauri::command]
async fn disconnect(app: AppHandle, ctx: Shared<'_>) -> Result<(), String> {
    core::disconnect(&app, ctx.inner(), None).await;
    Ok(())
}

#[tauri::command]
async fn ping(app: AppHandle, ctx: Shared<'_>, ids: Vec<String>) -> Result<(), String> {
    core::ping(app, ctx.inner().clone(), ids).await;
    Ok(())
}

#[tauri::command]
async fn locate_home(app: AppHandle, ctx: Shared<'_>) -> Result<Home, String> {
    if ctx.status.lock().unwrap().state != "off" {
        return Err("определить местоположение можно только при отключённом соединении".into());
    }
    let home = core::locate_home().await.ok_or("не удалось определить местоположение")?;
    ctx.data.lock().unwrap().settings.home = Some(home.clone());
    changed(&app, &ctx);
    Ok(home)
}

/// The share link of a server and its QR code as a grid of cells.
#[tauri::command]
fn share(ctx: Shared, id: String) -> Result<Value, String> {
    let d = ctx.data.lock().unwrap();
    let s = d.servers.iter().find(|s| s.id == id).ok_or("сервер не найден")?;
    let text = match (&s.uri, &s.config) {
        (Some(uri), _) => uri.clone(),
        (None, Some(c)) => serde_json::to_string_pretty(c).unwrap_or_default(),
        _ => return Err("у сервера нет ссылки".into()),
    };
    let qr = s.uri.as_ref().and_then(|u| qrcode::QrCode::new(u.as_bytes()).ok()).map(|code| {
        let cells: String = code.to_colors().iter().map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' }).collect();
        json!({"size": code.width(), "cells": cells})
    });
    Ok(json!({"text": text, "link": s.uri.is_some(), "qr": qr}))
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> bool {
    #[cfg(target_os = "android")]
    return android::open_url(&app, &url);
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        sys::open_url(&url)
    }
}

/// Programs to choose from in the exclusions: `[{id, name, icon?}]`. The id is what the
/// setting stores (an executable name, or a package name on Android).
#[tauri::command]
async fn list_apps(app: AppHandle) -> Result<Value, String> {
    #[cfg(target_os = "android")]
    {
        let handle = app.clone();
        return tokio::task::spawn_blocking(move || android::list_apps(&handle)).await.map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        let apps = tokio::task::spawn_blocking(sys::list_apps).await.map_err(|e| e.to_string())?;
        Ok(json!(apps.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()))
    }
}

#[tauri::command]
fn clipboard() -> String {
    sys::clipboard_text()
}

#[tauri::command]
fn get_logs(ctx: Shared) -> Vec<String> {
    ctx.logs.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
fn clear_logs(ctx: Shared) {
    ctx.logs.lock().unwrap().clear();
}

/// Restarts the app with administrator rights, which the all-traffic mode needs.
#[tauri::command]
async fn relaunch_admin(app: AppHandle, ctx: Shared<'_>) -> Result<(), String> {
    if !sys::relaunch_elevated("--elevated") {
        return Err("запуск с правами администратора отменён".into());
    }
    core::disconnect(&app, ctx.inner(), None).await;
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn show_main(app: &AppHandle) {
    #[cfg(desktop)]
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    let _ = app;
}

/// Development aid, debug builds only: a file `autotest.json` in the data folder
/// (`{"link": "...", "connect": true, "hold": 30}`) is imported at startup, the connection is made, and the
/// outcome is written next to it as `autotest-result.json`. This is how the Android build is
/// exercised on an emulator, where nobody is there to tap. With `"exclude": ["some.app"]` it then
/// connects a second time with those programs in the exclusions (`autotest-excluded.json`).
#[cfg(debug_assertions)]
async fn autotest(app: AppHandle, ctx: Arc<Ctx>) {
    let Ok(text) = std::fs::read_to_string(ctx.dir.join("autotest.json")) else { return };
    let Ok(job) = serde_json::from_str::<Value>(&text) else { return };
    let report = |v: Value| {
        let _ = std::fs::write(ctx.dir.join("autotest-result.json"), v.to_string());
    };
    report(json!({"stage": "started"}));
    let (mut servers, skipped) = subs::parse_body(job["link"].as_str().unwrap_or(""));
    subs::assign_ids(&mut servers, None);
    let Some(id) = servers.first().map(|s| s.id.clone()) else {
        return report(json!({"stage": "import", "error": "no server in the link", "skipped": skipped}));
    };
    {
        let mut d = ctx.data.lock().unwrap();
        d.servers.retain(|s| s.id != id);
        d.servers.extend(servers);
        d.settings.selected = Some(id.clone());
        d.settings.geo_lookup = false;
        apply_overrides(&mut d.settings, &job["settings"]);
    }
    changed(&app, &ctx);
    if job["connect"].as_bool().unwrap_or(false) {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let result = core::connect(app.clone(), ctx.clone(), id.clone()).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let status = serde_json::to_value(&*ctx.status.lock().unwrap()).unwrap_or_default();
        // how many programs the exclusions would offer to choose from
        let apps = list_apps(app.clone()).await.ok().and_then(|v| v.as_array().map(|a| a.len()));
        report(json!({"stage": "connect", "result": result.err(), "status": status, "apps": apps}));
        // "hold": seconds to stay connected before disconnecting again, to check the way back
        if let Some(hold) = job["hold"].as_u64() {
            tokio::time::sleep(std::time::Duration::from_secs(hold)).await;
            core::disconnect(&app, &ctx, None).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let status = serde_json::to_value(&*ctx.status.lock().unwrap()).unwrap_or_default();
            let _ = std::fs::write(ctx.dir.join("autotest-disconnect.json"), json!({"stage": "disconnect", "status": status}).to_string());
            if let Some(list) = job["exclude"].as_array() {
                tokio::time::sleep(std::time::Duration::from_secs(job["pause"].as_u64().unwrap_or(10))).await;
                ctx.data.lock().unwrap().settings.bypass_apps = list.iter().filter_map(|v| v.as_str().map(String::from)).collect();
                let result = core::connect(app.clone(), ctx.clone(), id).await;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let status = serde_json::to_value(&*ctx.status.lock().unwrap()).unwrap_or_default();
                let _ = std::fs::write(ctx.dir.join("autotest-excluded.json"), json!({"stage": "excluded", "result": result.err(), "status": status}).to_string());
                tokio::time::sleep(std::time::Duration::from_secs(hold)).await;
                core::disconnect(&app, &ctx, None).await;
            }
        }
    } else {
        report(json!({"stage": "imported"}));
    }
}

/// Debug hooks: `"settings": {...}` in a job file overrides those settings (mode, core, ports...).
#[cfg(debug_assertions)]
fn apply_overrides(settings: &mut Settings, overrides: &Value) {
    let Some(o) = overrides.as_object() else { return };
    let mut v = serde_json::to_value(&*settings).unwrap_or_default();
    for (k, x) in o {
        v[k] = x.clone();
    }
    if let Ok(s) = serde_json::from_value(v) {
        *settings = s;
    }
}

#[cfg(debug_assertions)]
fn rss_kb(pid: Option<u64>) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/proc/{}/status", pid?)).ok()?;
    text.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
}

/// Downloads `url` through the local SOCKS port; returns (bytes, milliseconds).
#[cfg(debug_assertions)]
async fn fetch_through(socks: u16, url: &str, secs: u64) -> Result<(u64, u64), String> {
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{socks}")).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder().proxy(proxy).timeout(std::time::Duration::from_secs(secs)).build().map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut n = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        n += chunk.len() as u64;
    }
    Ok((n, t.elapsed().as_millis() as u64))
}

/// Development aid, debug builds only: `bench.json` in the data folder runs connection cycles
/// and writes `bench-result.json` next to it.
/// `{"link": "<links or base64 list>", "pick": 0, "auto": false, "settings": {"mode": "ports", "core": "auto"},
///   "cycles": 20, "pause_ms": 500, "download_url": "...", "soak_secs": 0, "soak_download_url": "..."}`
/// Each cycle: connect (timings by stage), the core's memory, optionally a download through the
/// tunnel, disconnect. `soak_secs` then repeats connect/download/disconnect for that long and
/// counts failures and core crashes (`bench-soak.json`, rewritten after every round).
#[cfg(debug_assertions)]
async fn bench(app: AppHandle, ctx: Arc<Ctx>) {
    let path = ctx.dir.join("bench.json");
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let Ok(job) = serde_json::from_str::<Value>(&text) else { return };
    let _ = std::fs::rename(&path, ctx.dir.join("bench.started.json"));
    let out = |name: &str, v: &Value| {
        let _ = std::fs::write(ctx.dir.join(name), serde_json::to_vec_pretty(v).unwrap_or_default());
    };
    let (mut servers, _) = subs::parse_body(job["link"].as_str().unwrap_or(""));
    subs::assign_ids(&mut servers, None);
    if servers.is_empty() {
        return out("bench-result.json", &json!({"error": "no server in the link"}));
    }
    let auto = job["auto"].as_bool().unwrap_or(false);
    let pick = job["pick"].as_u64().unwrap_or(0) as usize;
    let id = if auto { "auto".to_string() } else { servers.get(pick).unwrap_or(&servers[0]).id.clone() };
    {
        let mut d = ctx.data.lock().unwrap();
        d.servers = servers;
        d.settings.geo_lookup = false;
        apply_overrides(&mut d.settings, &job["settings"]);
    }
    changed(&app, &ctx);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    if auto {
        core::ping(app.clone(), ctx.clone(), vec![]).await;
    }
    let socks = ctx.data.lock().unwrap().settings.socks_port;
    let pause = std::time::Duration::from_millis(job["pause_ms"].as_u64().unwrap_or(500));
    let mut cycles = vec![];
    for i in 0..job["cycles"].as_u64().unwrap_or(0) {
        *ctx.timing.lock().unwrap() = Value::Null;
        let result = core::connect(app.clone(), ctx.clone(), id.clone()).await;
        let mut row = ctx.timing.lock().unwrap().clone();
        if !row.is_object() {
            row = json!({});
        }
        row["cycle"] = json!(i);
        row["ok"] = json!(result.is_ok());
        if let Err(e) = result {
            row["error"] = json!(e);
        } else {
            let pid = row["pid"].as_u64();
            row["rss_kb"] = json!(rss_kb(pid));
            row["server"] = json!(ctx.status.lock().unwrap().server.clone());
            row["state"] = json!(ctx.status.lock().unwrap().state.clone());
            // without verify_first the check runs in the background: give it time, then record it
            if let Some(wait) = job["check_wait_ms"].as_u64() {
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                let s = ctx.status.lock().unwrap().clone();
                row["state_after_check"] = json!(s.state);
                row["check"] = json!(s.check);
                row["check_ms"] = json!(s.ms);
            }
            if let Some(url) = job["download_url"].as_str() {
                match fetch_through(socks, url, 60).await {
                    Ok((n, ms)) => {
                        row["download_bytes"] = json!(n);
                        row["download_ms"] = json!(ms);
                        row["download_mbit"] = json!((n as f64 * 8.0 / 1e6) / (ms.max(1) as f64 / 1000.0));
                    }
                    Err(e) => row["download_error"] = json!(e),
                }
                row["rss_kb_after_download"] = json!(rss_kb(pid));
            }
        }
        core::disconnect(&app, &ctx, None).await;
        cycles.push(row);
        out("bench-result.json", &json!({"done": false, "cycles": cycles}));
        tokio::time::sleep(pause).await;
    }
    let ok: Vec<&Value> = cycles.iter().filter(|c| c["ok"] == true).collect();
    let median = |key: &str| {
        let mut v: Vec<u64> = ok.iter().filter_map(|c| c[key].as_u64()).collect();
        v.sort();
        v.get(v.len() / 2).copied()
    };
    let summary = json!({"cycles": cycles.len(), "ok": ok.len(), "median_total_ms": median("total_ms"), "median_core_ms": median("core_ms"),
        "median_probe_ms": median("probe_ms"), "median_rss_kb": median("rss_kb"), "median_rss_kb_after_download": median("rss_kb_after_download")});
    out("bench-result.json", &json!({"done": true, "summary": summary, "cycles": cycles}));

    let soak_secs = job["soak_secs"].as_u64().unwrap_or(0);
    if soak_secs > 0 {
        let began = std::time::Instant::now();
        let url = job["soak_download_url"].as_str().or(job["download_url"].as_str()).map(String::from);
        let (mut rounds, mut fails, mut crashes, mut bytes, mut max_rss) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut errors: Vec<String> = vec![];
        while began.elapsed().as_secs() < soak_secs {
            rounds += 1;
            match core::connect(app.clone(), ctx.clone(), id.clone()).await {
                Err(e) => {
                    fails += 1;
                    if errors.len() < 50 {
                        errors.push(format!("{}s connect: {e}", began.elapsed().as_secs()));
                    }
                }
                Ok(()) => {
                    let pid = ctx.timing.lock().unwrap()["pid"].as_u64();
                    if let Some(url) = &url {
                        match fetch_through(socks, url, 60).await {
                            Ok((n, _)) => bytes += n,
                            Err(e) => {
                                fails += 1;
                                if errors.len() < 50 {
                                    errors.push(format!("{}s download: {e}", began.elapsed().as_secs()));
                                }
                            }
                        }
                    }
                    max_rss = max_rss.max(rss_kb(pid).unwrap_or(0));
                    let st = ctx.status.lock().unwrap().clone();
                    if st.error.as_deref().is_some_and(|e| e.contains("неожиданно")) || (st.state == "off" && st.error.is_some()) {
                        crashes += 1;
                    }
                }
            }
            core::disconnect(&app, &ctx, None).await;
            out("bench-soak.json", &json!({"done": false, "secs": began.elapsed().as_secs(), "rounds": rounds, "failures": fails,
                "core_crashes": crashes, "bytes": bytes, "max_rss_kb": max_rss, "errors": errors}));
            tokio::time::sleep(pause).await;
        }
        out("bench-soak.json", &json!({"done": true, "secs": began.elapsed().as_secs(), "rounds": rounds, "failures": fails,
            "core_crashes": crashes, "bytes": bytes, "max_rss_kb": max_rss, "errors": errors}));
    }
    if job["exit"].as_bool().unwrap_or(true) {
        app.exit(0);
    }
}

/// Refreshes subscriptions that are older than their update interval.
async fn auto_update(app: AppHandle, ctx: Arc<Ctx>) {
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    loop {
        let due: Vec<(String, String)> = {
            let d = ctx.data.lock().unwrap();
            let hours = d.settings.sub_update_hours;
            d.subs.iter().filter(|s| {
                let every = match (hours, s.update_hours) {
                    (0, _) => return false,
                    (h, 0) => h,
                    (h, p) => h.min(p),
                };
                now().saturating_sub(s.updated_at) >= every as u64 * 3600
            }).map(|s| (s.id.clone(), s.url.clone())).collect()
        };
        for (id, url) in due {
            let _ = refresh(&app, &ctx, &id, &url).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let elevated = args.iter().any(|a| a == "--elevated");
    let hidden = args.iter().any(|a| a == "--hidden");
    if elevated {
        // The instance that asked for elevation is still shutting down.
        std::thread::sleep(std::time::Duration::from_millis(900));
    }

    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
            .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])));
    }
    #[cfg(target_os = "android")]
    {
        builder = builder.plugin(tauri_plugin_quadra_vpn::init());
    }
    let app = builder
        .setup(move |app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            #[cfg(not(target_os = "android"))]
            let (xray, assets, singbox, singbox_bin) = {
                let res = app.path().resource_dir()?.join("resources");
                let d = res.join("xray");
                (d.clone(), d, res.join("sing-box"), res.join("sing-box").join(sys::SINGBOX_BIN))
            };
            // On Android both cores ship as native libraries of the package, and their data files
            // (Xray's .dat files, sing-box's rule-sets) are unpacked into the app's own folder by
            // the plugin.
            #[cfg(target_os = "android")]
            let (xray, assets, singbox, singbox_bin) = {
                let info = android::info(app.handle()).map_err(std::io::Error::other)?;
                sys::set_device(&info.model, &info.release);
                let native = PathBuf::from(&info.native_dir);
                let files = PathBuf::from(&info.files_dir);
                (native.clone(), files.join("xray"), files.join("sing-box"), native.join(sys::SINGBOX_BIN))
            };
            let tiling = sys::tiling_wm();
            let saved: Option<Data> = std::fs::read(dir.join("state.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
            let first_run = saved.is_none();
            let mut data = saved.unwrap_or_default();
            if data.hwid.is_empty() {
                data.hwid = subs::make_hwid();
            }
            if first_run && tiling {
                data.settings.close_to_tray = false; // most tiling setups have no tray to come back from
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(xray.join(sys::XRAY_BIN), std::fs::Permissions::from_mode(0o755));
                let _ = std::fs::set_permissions(&singbox_bin, std::fs::Permissions::from_mode(0o755));
            }
            let with_splash = cfg!(desktop) && !hidden && !tiling;
            let ctx = Arc::new(Ctx {
                dir, xray, assets, singbox, singbox_bin,
                data: Mutex::new(data),
                pings: Mutex::new(HashMap::new()),
                status: Mutex::new(Status { state: "off".into(), ..Default::default() }),
                logs: Mutex::new(VecDeque::new()),
                core: tokio::sync::Mutex::new(None),
                launch: tokio::sync::Mutex::new(()),
                gen: AtomicU64::new(0),
                core_ready: AtomicBool::new(false),
                verbose_core: AtomicBool::new(false),
                timing: Mutex::new(Value::Null),
                resolved: Mutex::new(HashMap::new()),
                hidden, elevated, tiling,
                splash: AtomicBool::new(with_splash),
                main_ready: AtomicBool::new(false),
            });
            core::restore_proxy(&ctx); // left over from a crash
            ctx.save();
            app.manage(ctx.clone());

            let handle = app.handle();
            #[cfg(desktop)]
            {
            let open = MenuItem::with_id(handle, "open", "Открыть", true, None::<&str>)?;
            let toggle = MenuItem::with_id(handle, "toggle", "Подключить / отключить", true, None::<&str>)?;
            let exit = MenuItem::with_id(handle, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(handle, &[&open, &toggle, &exit])?;
            let mut tray = TrayIconBuilder::with_id("main").tooltip("Quadra").menu(&menu).show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "toggle" => { let _ = app.emit("tray-toggle", ()); }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _ = tray.build(app); // a desktop without a tray is no reason not to start

            if with_splash {
                // The startup cube: a bare transparent window, nothing but the object itself.
                let mut builder = tauri::WebviewWindowBuilder::new(app, "splash", tauri::WebviewUrl::App("splash.html".into()));
                // WebView2 refuses a second window whose browser arguments differ from the first one's
                if let Some(args) = app.config().app.windows.first().and_then(|w| w.additional_browser_args.clone()) {
                    builder = builder.additional_browser_args(&args);
                }
                let built = builder
                    .title("Quadra")
                    .inner_size(320.0, 320.0)
                    .center()
                    .decorations(false)
                    .transparent(true)
                    .shadow(false)
                    .resizable(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .focused(false)
                    .visible(false)
                    .build();
                if built.is_err() {
                    ctx.splash.store(false, Ordering::SeqCst);
                }
            }
            }

            #[cfg(debug_assertions)]
            tauri::async_runtime::spawn(autotest(handle.clone(), ctx.clone()));
            #[cfg(debug_assertions)]
            tauri::async_runtime::spawn(bench(handle.clone(), ctx.clone()));
            tauri::async_runtime::spawn(auto_update(handle.clone(), ctx));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let ctx = window.state::<Arc<Ctx>>();
                if cfg!(desktop) && window.label() == "main" && ctx.data.lock().unwrap().settings.close_to_tray {
                    api.prevent_close();
                    #[cfg(desktop)]
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state, ready, splash, save_settings, add_subscription, refresh_subscription, delete_subscription, delete_server,
            import_text, connect, disconnect, ping, locate_home, share, open_url, list_apps, clipboard, get_logs, clear_logs, relaunch_admin, quit
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // The core dies with the job object; the system proxy has to be put back by hand.
            core::restore_proxy(&app.state::<Arc<Ctx>>());
        }
    });
}
