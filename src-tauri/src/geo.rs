//! Geo data: the v2ray `geosite` / `geoip` `.dat` files and sing-box rule-sets that routing
//! rules refer to (`geosite:tag`, `geoip:tag`, `ext:file.dat:tag`, `ruleset:name`).
//!
//! Everything lives in `geo/` under the app data dir, which is also Xray's asset folder: the
//! app's own `geosite.dat` / `geoip.dat` are copied there, next to community sets and the user's
//! files. Before a core starts, every reference in its config is checked against what is really
//! there. Known files that are missing get downloaded; references that still cannot be resolved
//! are dropped with a warning. A core never fails to start because of geo data (before 0.4.1 an
//! unknown category in a provider's config stopped Xray with "common/geodata: illegal domain
//! rule"). sing-box has no geosite at all: the categories it needs are converted into rule-sets
//! on demand and cached by file hash and tag. The UI contract is docs/geo-api.md.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SITE: &str = "geosite";
pub const IP: &str = "geoip";
pub const RULESET: &str = "ruleset";
const INDEX: &str = "index.json";
const DAY: u64 = 24 * 3600;
/// After a failed download the next try waits this long, not a whole day.
const RETRY: u64 = 3600;
const MAX_BYTES: usize = 300 << 20;

/// Tag (upper case, as Xray looks it up) → number of entries.
pub type Tags = BTreeMap<String, u32>;

// ---------------------------------------------------------------- built-in sets

/// A set the app knows by name. `aliases` are the other file names configs use for it in
/// `ext:` refs; an `ext:` ref to any of them makes the app fetch it. Nothing is fetched before
/// a config needs it (runetfreedom's geosite alone is 90 MB): by name, or because a plain
/// `geosite:` / `geoip:` tag that the files at hand lack is one of the set's `hints`.
pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    /// Empty: whatever the file turns out to be.
    pub kind: &'static str,
    pub url: &'static str,
    pub aliases: &'static [&'static str],
    pub hints: &'static [&'static str],
    pub description: &'static str,
    /// The copy that ships with the app (next to Xray) stands in until the first download.
    pub shipped: bool,
}

/// Checked on 16.11.2026. The defaults come from where Xray's own release takes them
/// (XTLS/Xray-rules-dat; its geoip.dat is geoip-lite). 3x-ui and its forks save runetfreedom's
/// files as geosite_RU.dat / geoip_RU.dat, the names provider configs use in `ext:` refs.
pub const CATALOG: &[Builtin] = &[
    Builtin {
        id: "default-geosite", name: "geosite.dat", kind: SITE, shipped: true,
        url: "https://github.com/XTLS/Xray-rules-dat/releases/latest/download/geosite.dat.gz",
        aliases: &[], hints: &[],
        description: "Основной geosite (из Xray): правила geosite:<тег> читают его первым",
    },
    Builtin {
        id: "default-geoip", name: "geoip.dat", kind: IP, shipped: true,
        url: "https://github.com/XTLS/Xray-rules-dat/releases/latest/download/geoip-lite.dat.gz",
        aliases: &[], hints: &[],
        description: "Основной geoip (из Xray): правила geoip:<тег> читают его первым",
    },
    Builtin {
        id: "ru-geosite", name: "geosite_RU.dat", kind: SITE, shipped: false,
        url: "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geosite.dat",
        aliases: &["geosite-ru.dat", "geosite_ru.dat", "russia-geosite.dat", "geosite-russia.dat", "runetfreedom-geosite.dat", "geosite_runetfreedom.dat"],
        hints: &["ru-blocked", "ru-blocked-all", "ru-available-only-inside", "antifilter-download", "antifilter-download-community",
                 "refilter", "category-ads-all", "win-spy", "win-update", "win-extra"],
        description: "runetfreedom: ru-blocked, ru-available-only-inside, refilter и все категории v2fly (около 90 МБ)",
    },
    Builtin {
        id: "ru-geoip", name: "geoip_RU.dat", kind: IP, shipped: false,
        url: "https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geoip.dat",
        aliases: &["geoip-ru.dat", "geoip_ru.dat", "russia-geoip.dat", "geoip-russia.dat", "runetfreedom-geoip.dat", "geoip_runetfreedom.dat"],
        hints: &["ru-blocked", "ru-blocked-community", "re-filter", "ru-whitelist", "ddos-guard", "yandex", "cloudflare", "cloudfront",
                 "facebook", "fastly", "google", "netflix", "telegram", "twitter"],
        description: "runetfreedom: ru-blocked, re-filter, ru-whitelist, страны и сети сервисов",
    },
    Builtin {
        id: "roscomvpn-geosite", name: "roscomvpn-geosite.dat", kind: SITE, shipped: false,
        url: "https://github.com/hydraponique/roscomvpn-geosite/releases/latest/download/geosite.dat",
        aliases: &["roscomvpn.dat", "geosite_roscomvpn.dat"], hints: &["whitelist", "category-geoblock-ru"],
        description: "RoscomVPN (как в Happ): whitelist, category-geoblock-ru",
    },
    Builtin {
        id: "roscomvpn-geoip", name: "roscomvpn-geoip.dat", kind: IP, shipped: false,
        url: "https://github.com/hydraponique/roscomvpn-geoip/releases/latest/download/geoip.dat",
        aliases: &["geoip_roscomvpn.dat"], hints: &["direct", "whitelist"],
        description: "RoscomVPN (как в Happ): direct, whitelist",
    },
    Builtin {
        id: "zkeen", name: "zkeen.dat", kind: SITE, shipped: false,
        url: "https://github.com/jameszeroX/zkeen-domains/releases/latest/download/zkeen.dat",
        aliases: &[], hints: &[],
        description: "zkeen-domains: domains, youtube, politic, other",
    },
    Builtin {
        id: "zkeenip", name: "zkeenip.dat", kind: IP, shipped: false,
        url: "https://github.com/jameszeroX/zkeen-ip/releases/latest/download/zkeenip.dat",
        aliases: &[], hints: &[],
        description: "zkeen-ip: ru, discord, telegram, youtube, cloudflare и другие",
    },
    Builtin {
        id: "zapret", name: "zapret.dat", kind: "", shipped: false,
        url: "https://github.com/kutovoys/ru_gov_zapret/releases/latest/download/zapret.dat",
        aliases: &[], hints: &[],
        description: "ru_gov_zapret (из документации Remnawave): zapret, zapret-zapad",
    },
    Builtin {
        id: "v2fly-geosite", name: "dlc.dat", kind: SITE, shipped: false,
        url: "https://github.com/v2fly/domain-list-community/releases/latest/download/dlc.dat",
        aliases: &["geosite_v2fly.dat", "v2fly-geosite.dat"], hints: &[],
        description: "v2fly domain-list-community: исходные категории geosite",
    },
    Builtin {
        id: "loyalsoldier-geosite", name: "geosite_loyalsoldier.dat", kind: SITE, shipped: false,
        url: "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat",
        aliases: &["loyalsoldier-geosite.dat"], hints: &[],
        description: "Loyalsoldier: geosite с китайскими списками и рекламой",
    },
    Builtin {
        id: "loyalsoldier-geoip", name: "geoip_loyalsoldier.dat", kind: IP, shipped: false,
        url: "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat",
        aliases: &["loyalsoldier-geoip.dat"], hints: &[],
        description: "Loyalsoldier: geoip со странами и сетями сервисов",
    },
];

fn builtin(id: &str) -> Option<&'static Builtin> {
    CATALOG.iter().find(|b| b.id == id)
}

// ---------------------------------------------------------------- stored entries

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// "geosite" | "geoip" | "ruleset"; empty for a built-in that was never downloaded.
    pub kind: String,
    /// "builtin" | "file" | "url"
    pub source: String,
    pub url: String,
    pub enabled: bool,
    pub auto_update: bool,
    pub size: u64,
    pub updated_at: u64,
    /// Last download attempt, successful or not.
    pub checked_at: u64,
    pub categories: u32,
    pub error: String,
    pub sha: String,
    pub etag: String,
    pub last_modified: String,
    /// The content came from `url`, not from the copy that ships with the app.
    pub downloaded: bool,
    /// Rule-sets: "binary" (.srs) or "source" (.json).
    pub format: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Index {
    files: Vec<Entry>,
    /// Categories converted for sing-box, by file name: made again right after an update.
    used: BTreeMap<String, Vec<String>>,
}

pub struct Geo {
    pub dir: PathBuf,
    /// Where the app's own geosite.dat / geoip.dat are.
    shipped: PathBuf,
    /// The sing-box binary, for compiling and reading rule-sets.
    singbox: Option<PathBuf>,
    files: Mutex<Vec<Entry>>,
    tags: Mutex<HashMap<String, Arc<Tags>>>,
    /// One download at a time per file; different files go in parallel.
    fetching: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    used: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    /// Rule-sets being made right now (by cache key), so two connections never make one twice.
    converting: Arc<Mutex<HashSet<String>>>,
    synced: AtomicBool,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_of(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Writes through a temporary file, so a reader never sees half a file.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.part"));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// A file name that is safe on every system and in an `ext:<name>:<tag>` ref.
fn safe_name(name: &str) -> String {
    let s: String = name.trim().chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    let s = s.trim_matches('.').chars().take(80).collect::<String>();
    if s.is_empty() { "geo".into() } else { s }
}

/// The name without its extension: `ruleset:<stem>`, rule-set tags in sing-box.
pub fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

impl Geo {
    pub fn open(dir: PathBuf, shipped: PathBuf, singbox: Option<PathBuf>) -> Geo {
        let _ = std::fs::create_dir_all(dir.join("rules"));
        let _ = std::fs::create_dir_all(dir.join("tags"));
        let index = std::fs::read(dir.join(INDEX)).ok().and_then(|b| serde_json::from_slice::<Index>(&b).ok()).unwrap_or_default();
        let used: HashMap<String, HashSet<String>> = index.used.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect();
        let saved = index.files;
        let mut files = vec![];
        for b in CATALOG {
            let mut e = saved.iter().find(|e| e.id == b.id).cloned()
                .unwrap_or_else(|| Entry { id: b.id.into(), enabled: true, auto_update: true, ..Default::default() });
            e.name = b.name.into();
            if !b.kind.is_empty() {
                e.kind = b.kind.into();
            }
            e.source = "builtin".into();
            e.url = b.url.into();
            files.push(e);
        }
        files.extend(saved.into_iter().filter(|e| e.source != "builtin" && !e.name.is_empty()));
        Geo { dir, shipped, singbox: singbox.filter(|p| p.is_file()), files: Mutex::new(files),
              tags: Mutex::new(HashMap::new()), fetching: Mutex::new(HashMap::new()), used: Arc::new(Mutex::new(used)), converting: Arc::default(),
              synced: AtomicBool::new(false) }
    }

    /// Writes the list (and the categories in use) to disk.
    pub fn save(&self) {
        let used = self.used.lock().unwrap().iter().map(|(k, v)| {
            let mut v: Vec<String> = v.iter().cloned().collect();
            v.sort();
            (k.clone(), v)
        }).collect();
        let index = Index { files: self.files.lock().unwrap().clone(), used };
        let _ = write_atomic(&self.dir.join(INDEX), &serde_json::to_vec_pretty(&index).unwrap_or_default());
    }

    fn entry(&self, id: &str) -> Option<Entry> {
        self.files.lock().unwrap().iter().find(|e| e.id == id).cloned()
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut Entry)) {
        if let Some(e) = self.files.lock().unwrap().iter_mut().find(|e| e.id == id) {
            f(e);
        }
        self.save();
    }

    fn path(&self, e: &Entry) -> PathBuf {
        self.dir.join(&e.name)
    }

    /// What the UI gets (docs/geo-api.md, GeoFile).
    pub fn view(&self, e: &Entry) -> Value {
        let path = self.path(e);
        let present = path.is_file();
        let mut v = json!({
            "id": e.id, "name": e.name, "kind": if e.kind.is_empty() { SITE } else { e.kind.as_str() },
            "source": e.source, "path": if present { path.to_string_lossy().to_string() } else { String::new() },
            "size": if present { e.size } else { 0 }, "updated_at": if present { e.updated_at } else { 0 },
            "categories": if present { e.categories } else { 0 }, "auto_update": e.auto_update, "enabled": e.enabled,
        });
        if !e.url.is_empty() {
            v["url"] = json!(e.url);
        }
        if !e.error.is_empty() {
            v["error"] = json!(e.error);
        }
        if let Some(b) = builtin(&e.id) {
            v["aliases"] = json!(b.aliases);
            v["description"] = json!(b.description);
        }
        v
    }

    pub fn list(&self) -> Vec<Value> {
        let files = self.files.lock().unwrap().clone();
        files.iter().map(|e| self.view(e)).collect()
    }

    fn view_of(&self, id: &str) -> Result<Value, String> {
        self.entry(id).map(|e| self.view(&e)).ok_or_else(|| "такого файла нет".into())
    }

    /// Copies the app's own geosite.dat / geoip.dat into the folder, when they are not there
    /// or the app brought newer ones than the last download.
    pub fn sync_defaults(&self) {
        for b in CATALOG.iter().filter(|b| b.shipped) {
            let src = self.shipped.join(b.name);
            let dst = self.dir.join(b.name);
            let Ok(meta) = std::fs::metadata(&src) else { continue };
            let Some(e) = self.entry(b.id) else { continue };
            let shipped_at = mtime(&src);
            let have = std::fs::metadata(&dst).map(|m| m.len()).ok();
            let stale = match have {
                None => true,
                Some(len) => e.sha.is_empty() || (!e.downloaded && len != meta.len()) || (e.downloaded && shipped_at > e.updated_at),
            };
            if !stale {
                continue;
            }
            let Ok(data) = std::fs::read(&src) else { continue };
            let Ok((kind, tags)) = scan_dat(&data) else { continue };
            if write_atomic(&dst, &data).is_err() {
                continue;
            }
            let sha = sha_of(&data);
            self.remember_tags(&sha, tags.clone());
            self.edit(b.id, |e| {
                e.kind = kind.into();
                e.size = data.len() as u64;
                e.sha = sha;
                e.categories = tags.len() as u32;
                e.updated_at = shipped_at.max(1);
                e.downloaded = false;
                e.etag.clear();
                e.last_modified.clear();
                e.error.clear();
            });
        }
        self.synced.store(true, Ordering::SeqCst);
    }

    fn remember_tags(&self, sha: &str, tags: Tags) -> Arc<Tags> {
        let tags = Arc::new(tags);
        let _ = std::fs::write(self.dir.join("tags").join(format!("{sha}.json")), serde_json::to_vec(&*tags).unwrap_or_default());
        self.tags.lock().unwrap().insert(sha.to_string(), tags.clone());
        tags
    }

    /// The categories of a `.dat` file: from memory, from the cache next to it, or by reading it.
    fn tags_of(&self, e: &Entry) -> Option<Arc<Tags>> {
        if !e.sha.is_empty() {
            if let Some(t) = self.tags.lock().unwrap().get(&e.sha) {
                return Some(t.clone());
            }
            if let Some(t) = std::fs::read(self.dir.join("tags").join(format!("{}.json", e.sha))).ok()
                .and_then(|b| serde_json::from_slice::<Tags>(&b).ok()) {
                let t = Arc::new(t);
                self.tags.lock().unwrap().insert(e.sha.clone(), t.clone());
                return Some(t);
            }
        }
        let data = std::fs::read(self.path(e)).ok()?;
        let (_, tags) = scan_dat(&data).ok()?;
        let sha = sha_of(&data);
        if sha != e.sha {
            self.edit(&e.id, |x| x.sha = sha.clone());
        }
        Some(self.remember_tags(&sha, tags))
    }

    /// A name no other file and no built-in uses.
    fn unique_name(&self, wanted: &str) -> String {
        let files = self.files.lock().unwrap();
        let taken = |n: &str| {
            let n = n.to_ascii_lowercase();
            n == INDEX || n == "rules" || n == "tags"
                || files.iter().any(|e| e.name.eq_ignore_ascii_case(&n))
                || CATALOG.iter().any(|b| b.name.eq_ignore_ascii_case(&n) || b.aliases.iter().any(|a| a.eq_ignore_ascii_case(&n)))
        };
        if !taken(wanted) {
            return wanted.to_string();
        }
        let (base, ext) = match wanted.rfind('.') {
            Some(i) if i > 0 => (&wanted[..i], &wanted[i..]),
            _ => (wanted, ""),
        };
        (2..).map(|n| format!("{base}-{n}{ext}")).find(|n| !taken(n)).unwrap()
    }

    fn new_id(&self, seed: &str) -> String {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        format!("u-{}", &sha_of(format!("{seed}-{t}").as_bytes())[..8])
    }

    /// Stores new content for a new or an existing entry. Runs on a blocking thread.
    fn store(&self, id: Option<&str>, name_hint: &str, data: Vec<u8>, make: impl FnOnce(&str, &Detected) -> Entry,
             etag: String, last_modified: String) -> Result<Entry, String> {
        let data = gunzip(data)?;
        let det = detect(&data)?;
        let mut entry = match id.and_then(|id| self.entry(id)) {
            Some(e) => {
                let expected = if e.kind.is_empty() { builtin(&e.id).map(|b| b.kind).unwrap_or("") } else { e.kind.as_str() };
                if !expected.is_empty() && expected != det.kind {
                    return Err(format!("ожидался {expected}, а пришёл {}", det.kind));
                }
                e
            }
            None => {
                let mut name = safe_name(name_hint);
                let ext_ok = match det.kind {
                    RULESET => name.ends_with(".srs") || name.ends_with(".json"),
                    _ => name.contains('.'),
                };
                if !ext_ok {
                    name.push_str(match (det.kind, det.format) {
                        (RULESET, "binary") => ".srs",
                        (RULESET, _) => ".json",
                        _ => ".dat",
                    });
                }
                make(&self.unique_name(&name), &det)
            }
        };
        let path = self.path(&entry);
        write_atomic(&path, &data).map_err(|e| format!("не удалось сохранить файл: {e}"))?;
        let sha = sha_of(&data);
        if !entry.sha.is_empty() && entry.sha != sha {
            self.forget(&entry.sha);
        }
        if let Some(t) = det.tags {
            self.remember_tags(&sha, t);
        }
        entry.kind = det.kind.into();
        entry.format = det.format.into();
        entry.size = data.len() as u64;
        entry.sha = sha;
        entry.categories = det.categories;
        entry.updated_at = now();
        entry.checked_at = entry.updated_at;
        entry.error.clear();
        entry.etag = etag;
        entry.last_modified = last_modified;
        entry.downloaded = !entry.url.is_empty();
        {
            let mut files = self.files.lock().unwrap();
            match files.iter_mut().find(|e| e.id == entry.id) {
                Some(e) => *e = entry.clone(),
                None => files.push(entry.clone()),
            }
        }
        self.save();
        Ok(entry)
    }

    /// Drops the caches made from a file's old content.
    fn forget(&self, sha: &str) {
        self.tags.lock().unwrap().remove(sha);
        let _ = std::fs::remove_file(self.dir.join("tags").join(format!("{sha}.json")));
        if sha.len() >= 12 {
            if let Ok(list) = std::fs::read_dir(self.dir.join("rules")) {
                for f in list.flatten() {
                    if f.file_name().to_string_lossy().starts_with(&sha[..12]) {
                        let _ = std::fs::remove_file(f.path());
                    }
                }
            }
        }
    }

    pub fn add_file(&self, path: &Path) -> Result<Value, String> {
        let data = std::fs::read(path).map_err(|e| format!("не удалось прочитать файл: {e}"))?;
        let hint = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.add_bytes(&hint, data)
    }

    /// A file's content and its name (Android hands over content, not a path).
    pub fn add_bytes(&self, name: &str, data: Vec<u8>) -> Result<Value, String> {
        let hint = name.strip_suffix(".gz").unwrap_or(name).to_string();
        let id = self.new_id(&hint);
        let e = self.store(None, &hint, data, |name, _| Entry {
            id: id.clone(), name: name.into(), source: "file".into(), enabled: true, ..Default::default()
        }, String::new(), String::new())?;
        Ok(self.view(&e))
    }

    pub async fn add_url(self: &Arc<Self>, url: &str, auto_update: bool, via: Option<u16>) -> Result<Value, String> {
        let url = url.trim().to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("нужна ссылка http:// или https://".into());
        }
        let existing = self.files.lock().unwrap().iter().find(|e| e.url == url).map(|e| e.id.clone());
        if let Some(id) = existing {
            self.edit(&id, |e| e.auto_update = auto_update);
            let _ = self.update_one(&id, via).await;
            return self.view_of(&id);
        }
        let dl = download(&url, "", "", via).await?.ok_or("сервер не отдал файл")?;
        let hint = url.split(['?', '#']).next().unwrap_or("").rsplit('/').find(|s| !s.is_empty()).unwrap_or("geo.dat").to_string();
        let hint = hint.strip_suffix(".gz").unwrap_or(&hint).to_string();
        let id = self.new_id(&url);
        let me = self.clone();
        let e = tokio::task::spawn_blocking(move || {
            me.store(None, &hint, dl.bytes, |name, _| Entry {
                id: id.clone(), name: name.into(), source: "url".into(), url: url.clone(), enabled: true, auto_update, ..Default::default()
            }, dl.etag, dl.last_modified)
        }).await.map_err(|e| e.to_string())??;
        Ok(self.view(&e))
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        if builtin(id).is_some() {
            return Err("встроенный набор нельзя удалить, его можно только выключить".into());
        }
        let e = self.entry(id).ok_or("такого файла нет")?;
        let _ = std::fs::remove_file(self.path(&e));
        if !e.sha.is_empty() {
            self.forget(&e.sha);
        }
        self.files.lock().unwrap().retain(|x| x.id != id);
        self.save();
        Ok(())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Value, String> {
        self.entry(id).ok_or("такого файла нет")?;
        self.edit(id, |e| e.enabled = enabled);
        self.view_of(id)
    }

    /// Downloads one entry again (with ETag / If-Modified-Since). Errors also land in the entry.
    pub async fn update_one(self: &Arc<Self>, id: &str, via: Option<u16>) -> Result<(), String> {
        self.fetch(id, via, false).await
    }

    async fn fetch(self: &Arc<Self>, id: &str, via: Option<u16>, only_missing: bool) -> Result<(), String> {
        let lock = self.fetching.lock().unwrap().entry(id.to_string()).or_default().clone();
        let _one = lock.lock().await;
        if only_missing && self.entry(id).is_some_and(|e| self.path(&e).is_file() && !e.sha.is_empty()) {
            return Ok(()); // fetched meanwhile by whoever held the lock
        }
        let e = self.entry(id).ok_or("такого файла нет")?;
        if e.url.is_empty() {
            if builtin(id).is_some_and(|b| b.shipped) {
                let me = self.clone();
                let _ = tokio::task::spawn_blocking(move || me.sync_defaults()).await;
            }
            return Ok(());
        }
        let present = self.path(&e).is_file() && !e.sha.is_empty();
        let (etag, lm) = if present { (e.etag.as_str(), e.last_modified.as_str()) } else { ("", "") };
        let got = download(&e.url, etag, lm, via).await;
        match got {
            Err(err) => {
                self.edit(id, |x| {
                    x.error = err.clone();
                    x.checked_at = now();
                });
                Err(err)
            }
            Ok(None) => {
                self.edit(id, |x| {
                    x.error.clear();
                    x.checked_at = now();
                    x.updated_at = x.checked_at;
                });
                Ok(())
            }
            Ok(Some(dl)) => {
                let me = self.clone();
                let id2 = id.to_string();
                let old_sha = e.sha.clone();
                let r = tokio::task::spawn_blocking(move || {
                    me.store(Some(&id2), "", dl.bytes, |_, _| Entry::default(), dl.etag, dl.last_modified)
                }).await.map_err(|e| e.to_string()).and_then(|r| r);
                match &r {
                    Err(err) => self.edit(id, |x| {
                        x.error = err.clone();
                        x.checked_at = now();
                    }),
                    Ok(new) if !old_sha.is_empty() && new.sha != old_sha => {
                        // the categories in use, for the new content, before a connection needs them
                        let (me, name) = (self.clone(), new.name.clone());
                        tokio::task::spawn_blocking(move || me.reconvert(&name));
                    }
                    Ok(_) => {}
                }
                r.map(|_| ())
            }
        }
    }

    /// Converts again, for a file's new content, the categories sing-box used from it.
    pub fn reconvert(&self, name: &str) {
        let refs: Vec<String> = self.used.lock().unwrap().get(name).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        if refs.is_empty() {
            return;
        }
        let r = self.resolver();
        for x in refs {
            let _ = r.singbox_set(&x);
        }
    }

    /// `geo_update`: one entry, or every enabled one that has an address.
    pub async fn update(self: &Arc<Self>, id: Option<&str>, via: Option<u16>) -> Vec<Value> {
        let ids: Vec<String> = match id {
            Some(id) => vec![id.to_string()],
            None => self.files.lock().unwrap().iter().filter(|e| e.enabled && !e.url.is_empty()).map(|e| e.id.clone()).collect(),
        };
        for id in ids {
            let _ = self.update_one(&id, via).await;
        }
        self.list()
    }

    /// Entries due for a background download.
    pub fn due(&self) -> Vec<String> {
        let t = now();
        self.files.lock().unwrap().iter().filter(|e| {
            if !e.enabled || e.url.is_empty() {
                return false;
            }
            let present = self.path(e).is_file() && !e.sha.is_empty();
            let wait = if e.error.is_empty() { DAY } else { RETRY };
            if !present {
                // never fetched: built-ins wait until a config needs them; an address the user
                // (or a provider's config) added is tried again
                return builtin(&e.id).is_none() && t.saturating_sub(e.checked_at) >= if e.checked_at == 0 { 0 } else { RETRY };
            }
            e.auto_update && t.saturating_sub(e.updated_at.max(e.checked_at)) >= wait
        }).map(|e| e.id.clone()).collect()
    }

    /// `geo_categories`.
    pub fn categories(&self, id: &str, query: Option<&str>, limit: Option<usize>) -> Result<Vec<Value>, String> {
        let e = self.entry(id).ok_or("такого файла нет")?;
        if !self.path(&e).is_file() {
            return Err("файл ещё не скачан".into());
        }
        if e.kind == RULESET {
            let count = self.ruleset_json(&e).map(|j| ruleset_count(&j)).unwrap_or(0);
            return Ok(vec![json!({"tag": stem(&e.name).to_lowercase(), "count": count})]);
        }
        let tags = self.tags_of(&e).ok_or("файл не читается")?;
        let q = query.unwrap_or("").trim().to_lowercase();
        let mut out: Vec<(String, u32)> = tags.iter().map(|(t, n)| (t.to_lowercase(), *n)).filter(|(t, _)| q.is_empty() || t.contains(&q)).collect();
        out.sort();
        Ok(out.into_iter().take(limit.unwrap_or(500)).map(|(tag, count)| json!({"tag": tag, "count": count})).collect())
    }

    /// A rule-set's source form: the file itself, or what sing-box decompiles it to.
    fn ruleset_json(&self, e: &Entry) -> Option<Value> {
        let path = self.path(e);
        if e.format != "binary" {
            return std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        }
        let out = self.dir.join("rules").join(format!("{}-decompiled.json", e.sha.get(..12).unwrap_or("x")));
        if !out.is_file() {
            let bin = self.singbox.as_ref()?;
            if !run(bin, &["rule-set", "decompile", "--output", &out.to_string_lossy(), &path.to_string_lossy()]) {
                return None;
            }
        }
        std::fs::read(&out).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Xray's own "geodata" section names files with addresses: they become url entries here,
    /// so the refs to them resolve (Xray itself would fail before it got to download them).
    pub fn adopt_assets(&self, cfg: &Value) {
        let mut added = false;
        for a in cfg["geodata"]["assets"].as_array().into_iter().flatten() {
            let (Some(url), Some(file)) = (a["url"].as_str(), a["file"].as_str()) else { continue };
            if !url.starts_with("https://") || file.trim().is_empty() {
                continue;
            }
            let name = safe_name(file.trim_end_matches(".gz"));
            let known = self.files.lock().unwrap().iter().any(|e| e.url == url || e.name.eq_ignore_ascii_case(&name))
                || CATALOG.iter().any(|b| b.aliases.iter().any(|x| x.eq_ignore_ascii_case(&name)));
            if known {
                continue;
            }
            let id = self.new_id(url);
            self.files.lock().unwrap().push(Entry { id, name, source: "url".into(), url: url.into(), enabled: true, auto_update: true, ..Default::default() });
            added = true;
        }
        if added {
            self.save();
        }
    }

    /// Before a connection: downloads what the refs need and is known but not here yet.
    /// Bounded by `budget`; whatever does not make it in time is dropped from the config later.
    pub async fn ensure(self: &Arc<Self>, refs: Vec<String>, via: Option<u16>, budget: Duration) {
        let me = self.clone();
        let ids = tokio::task::spawn_blocking(move || me.plan_fetch(&refs)).await.unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        // all at once (a small list need not wait behind a 90 MB one); what does not finish in
        // time goes on in the background, for the next connection
        let jobs: Vec<_> = ids.into_iter().map(|id| {
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.fetch(&id, via, true).await;
            })
        }).collect();
        let _ = tokio::time::timeout(budget, async move {
            for j in jobs {
                let _ = j.await;
            }
        }).await;
    }

    fn plan_fetch(&self, refs: &[String]) -> Vec<String> {
        if !self.synced.load(Ordering::SeqCst) {
            self.sync_defaults();
        }
        let files = self.files.lock().unwrap().clone();
        let present = |e: &Entry| self.path(e).is_file() && !e.sha.is_empty();
        let mut ids: Vec<String> = vec![];
        let mut want = |id: &str| if !ids.iter().any(|i| i == id) { ids.push(id.to_string()) };
        let by_name = |n: &str| files.iter().find(|e| e.name.eq_ignore_ascii_case(n)
            || builtin(&e.id).is_some_and(|b| b.aliases.iter().any(|a| a.eq_ignore_ascii_case(n))));
        for r in refs.iter().filter_map(|r| parse_ref(r)) {
            let (file, kind, tag) = match &r {
                Ref::Site { file, tag, .. } => (file.as_deref(), SITE, tag),
                Ref::Ip { file, tag, .. } => (file.as_deref(), IP, tag),
                Ref::Ext { file, tag, .. } => (Some(file.as_str()), "", tag),
                Ref::RuleSet { .. } => continue,
            };
            match file {
                Some(f) => {
                    if let Some(e) = by_name(f) {
                        if e.enabled && !e.url.is_empty() && !present(e) {
                            want(&e.id);
                        }
                    }
                }
                None => {
                    let upper = tag.to_uppercase();
                    let found = files.iter().filter(|e| e.enabled && e.kind == kind && present(e))
                        .any(|e| self.tags_of(e).is_some_and(|t| t.contains_key(&upper)));
                    if !found {
                        // the first set known to carry the tag
                        let carrier = files.iter().filter(|e| e.enabled && !present(e) && !e.url.is_empty())
                            .find(|e| builtin(&e.id).is_some_and(|b| b.kind == kind && b.hints.iter().any(|h| h.eq_ignore_ascii_case(tag))));
                        if let Some(e) = carrier {
                            want(&e.id);
                        }
                    }
                }
            }
        }
        ids
    }

    /// What is usable right now, for building one config. Runs on a blocking thread.
    pub fn resolver(&self) -> Resolver {
        if !self.synced.load(Ordering::SeqCst) {
            self.sync_defaults();
        }
        let files = self.files.lock().unwrap().clone();
        let mut avail = vec![];
        for e in files.iter().filter(|e| e.enabled) {
            let path = self.path(e);
            if !path.is_file() || e.kind.is_empty() {
                continue;
            }
            let tags = if e.kind == RULESET {
                let mut t = Tags::new();
                t.insert(stem(&e.name).to_uppercase(), 1);
                Arc::new(t)
            } else {
                match self.tags_of(e) {
                    Some(t) => t,
                    None => continue,
                }
            };
            let sha = self.entry(&e.id).map(|x| x.sha).unwrap_or_default();
            let ruleset = if e.kind == RULESET { self.ruleset_json(e) } else { None };
            avail.push(Avail {
                name: e.name.clone(),
                aliases: builtin(&e.id).map(|b| b.aliases.iter().map(|a| a.to_string()).collect()).unwrap_or_default(),
                kind: e.kind.clone(), path, sha, tags, format: e.format.clone(), ruleset,
                default: builtin(&e.id).is_some_and(|b| b.shipped),
            });
        }
        Resolver { dir: self.dir.clone(), singbox: self.singbox.clone(), files: avail, bytes: Mutex::new(HashMap::new()), used: self.used.clone(),
                   converting: self.converting.clone(), convert: AtomicBool::new(true) }
    }
}

// ---------------------------------------------------------------- downloads

pub struct Download {
    pub bytes: Vec<u8>,
    pub etag: String,
    pub last_modified: String,
}

/// GET with conditional headers; `Ok(None)` when the server says nothing changed. Through the
/// running core first when there is one (GitHub is slow or cut in some places), then directly.
pub async fn download(url: &str, etag: &str, last_modified: &str, via: Option<u16>) -> Result<Option<Download>, String> {
    let mut last = String::new();
    let routes: Vec<Option<u16>> = match via {
        Some(p) => vec![Some(p), None],
        None => vec![None],
    };
    for route in routes {
        match fetch_once(url, etag, last_modified, route).await {
            Ok(r) => return Ok(r),
            Err(e) => last = e,
        }
    }
    Err(last)
}

async fn fetch_once(url: &str, etag: &str, last_modified: &str, via: Option<u16>) -> Result<Option<Download>, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(format!("Quadra/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(240));
    b = match via {
        Some(p) => b.proxy(reqwest::Proxy::all(format!("socks5h://127.0.0.1:{p}")).map_err(|e| e.to_string())?),
        None => b.no_proxy(),
    };
    let client = b.build().map_err(|e| e.to_string())?;
    let mut req = client.get(url);
    if !etag.is_empty() {
        req = req.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    if !last_modified.is_empty() {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, last_modified);
    }
    let mut resp = req.send().await.map_err(|e| format!("не удалось скачать: {}", short_error(&e)))?;
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("сервер ответил {}", resp.status().as_u16()));
    }
    if resp.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return Err("файл больше 300 МБ".into());
    }
    let header = |name: reqwest::header::HeaderName| resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (etag, last_modified) = (header(reqwest::header::ETAG), header(reqwest::header::LAST_MODIFIED));
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("не удалось скачать: {}", short_error(&e)))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BYTES {
            return Err("файл больше 300 МБ".into());
        }
    }
    if bytes.is_empty() {
        return Err("сервер прислал пустой файл".into());
    }
    Ok(Some(Download { bytes, etag, last_modified }))
}

fn short_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "время ожидания вышло".into();
    }
    if e.is_connect() {
        return "нет соединения с сервером".into();
    }
    let mut s = e.to_string();
    s.truncate(160);
    s
}

fn gunzip(data: Vec<u8>) -> Result<Vec<u8>, String> {
    if data.len() < 2 || data[0] != 0x1f || data[1] != 0x8b {
        return Ok(data);
    }
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&data[..]).take(MAX_BYTES as u64 + 1).read_to_end(&mut out).map_err(|e| format!("архив повреждён: {e}"))?;
    if out.len() > MAX_BYTES {
        return Err("файл больше 300 МБ".into());
    }
    Ok(out)
}

/// Runs sing-box for a rule-set job, at most 60 s, without a console window.
fn run(bin: &Path, args: &[&str]) -> bool {
    let mut c = std::process::Command::new(bin);
    c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = c.spawn() else { return false };
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

// ---------------------------------------------------------------- protobuf

struct Field<'a> {
    num: u64,
    wire: u8,
    val: u64,
    data: &'a [u8],
}

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        v |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn read_field<'a>(buf: &'a [u8], pos: &mut usize) -> Option<Field<'a>> {
    let key = varint(buf, pos)?;
    let (num, wire) = (key >> 3, (key & 7) as u8);
    if num == 0 {
        return None;
    }
    let skip = |pos: &mut usize, n: usize| -> Option<()> {
        let end = pos.checked_add(n)?;
        if end > buf.len() {
            return None;
        }
        *pos = end;
        Some(())
    };
    match wire {
        0 => Some(Field { num, wire, val: varint(buf, pos)?, data: &[] }),
        1 => skip(pos, 8).map(|_| Field { num, wire, val: 0, data: &[] }),
        5 => skip(pos, 4).map(|_| Field { num, wire, val: 0, data: &[] }),
        2 => {
            let len = varint(buf, pos)? as usize;
            let start = *pos;
            skip(pos, len)?;
            Some(Field { num, wire, val: len as u64, data: &buf[start..*pos] })
        }
        _ => None,
    }
}

/// The fields of one message; `broken` is set when it does not parse to the end.
struct Fields<'a> {
    buf: &'a [u8],
    pos: usize,
    broken: bool,
}

impl<'a> Iterator for Fields<'a> {
    type Item = Field<'a>;
    fn next(&mut self) -> Option<Field<'a>> {
        if self.broken || self.pos >= self.buf.len() {
            return None;
        }
        let f = read_field(self.buf, &mut self.pos);
        if f.is_none() {
            self.broken = true;
        }
        f
    }
}

fn fields(buf: &[u8]) -> Fields<'_> {
    Fields { buf, pos: 0, broken: false }
}

/// A GeoSiteList or GeoIPList: which of the two, and its tags with entry counts.
/// Both are `repeated Entry = 1`, entries `{ string code = 1; repeated Item = 2; ... }`; a
/// geosite item has a string value in field 2, a geoip item 4 or 16 bytes of address in field 1.
pub fn scan_dat(buf: &[u8]) -> Result<(&'static str, Tags), String> {
    let bad = || "не похоже на файл geosite/geoip (.dat)".to_string();
    let mut tags = Tags::new();
    let (mut site, mut ip, mut sampled) = (0u32, 0u32, 0u32);
    let mut top = fields(buf);
    for f in &mut top {
        if f.num != 1 || f.wire != 2 {
            return Err(bad());
        }
        let mut code = String::new();
        let mut count = 0u32;
        let mut inner = fields(f.data);
        for g in &mut inner {
            match (g.num, g.wire) {
                (1, 2) => code = String::from_utf8_lossy(g.data).to_uppercase(),
                (2, 2) => {
                    count += 1;
                    if sampled < 400 {
                        sampled += 1;
                        for h in fields(g.data) {
                            match (h.num, h.wire) {
                                (1, 2) if h.data.len() == 4 || h.data.len() == 16 => {
                                    ip += 1;
                                    break;
                                }
                                (1, 0) | (2, 2) => {
                                    site += 1;
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if inner.broken || code.is_empty() || code.len() > 200 {
            return Err(bad());
        }
        *tags.entry(code).or_default() += count;
    }
    if top.broken || tags.is_empty() {
        return Err(bad());
    }
    let kind = if ip > site { IP } else if site > 0 { SITE } else if tags.keys().any(|t| t.len() == 2) { IP } else { SITE };
    Ok((kind, tags))
}

/// The first entry with this code (as Xray picks it).
fn find_entry<'a>(buf: &'a [u8], code: &str) -> Option<&'a [u8]> {
    let code = code.to_uppercase();
    for f in fields(buf) {
        if f.num != 1 || f.wire != 2 {
            return None;
        }
        for g in fields(f.data) {
            if g.num == 1 && g.wire == 2 {
                if g.data.eq_ignore_ascii_case(code.as_bytes()) {
                    return Some(f.data);
                }
                break;
            }
        }
    }
    None
}

/// One domain of a geosite category: v2ray type (0 keyword, 1 regex, 2 domain and its
/// subdomains, 3 full) and value.
pub type SiteItem = (u64, String);

/// The domains of a geosite category that carry every one of `attrs` (lower case).
pub fn site_items(buf: &[u8], code: &str, attrs: &[String]) -> Option<Vec<SiteItem>> {
    let entry = find_entry(buf, code)?;
    let mut out = vec![];
    for g in fields(entry) {
        if g.num != 2 || g.wire != 2 {
            continue;
        }
        let (mut kind, mut value, mut keys) = (0u64, String::new(), Vec::<String>::new());
        for h in fields(g.data) {
            match (h.num, h.wire) {
                (1, 0) => kind = h.val,
                (2, 2) => value = String::from_utf8_lossy(h.data).into_owned(),
                (3, 2) if !attrs.is_empty() => {
                    for k in fields(h.data) {
                        if k.num == 1 && k.wire == 2 {
                            keys.push(String::from_utf8_lossy(k.data).to_lowercase());
                        }
                    }
                }
                _ => {}
            }
        }
        if value.is_empty() || !attrs.iter().all(|a| keys.contains(a)) {
            continue;
        }
        out.push((kind, value));
    }
    Some(out)
}

/// The networks of a geoip category, and whether the entry is meant reversed.
pub fn ip_items(buf: &[u8], code: &str) -> Option<(Vec<String>, bool)> {
    let entry = find_entry(buf, code)?;
    let (mut out, mut reverse) = (vec![], false);
    for g in fields(entry) {
        match (g.num, g.wire) {
            (2, 2) => {
                let (mut ip, mut prefix) = (None, 0u64);
                for h in fields(g.data) {
                    match (h.num, h.wire) {
                        (1, 2) => ip = Some(h.data),
                        (2, 0) => prefix = h.val,
                        _ => {}
                    }
                }
                let addr = match ip {
                    Some(b) if b.len() == 4 => std::net::IpAddr::from([b[0], b[1], b[2], b[3]]),
                    Some(b) if b.len() == 16 => {
                        let mut a = [0u8; 16];
                        a.copy_from_slice(b);
                        std::net::IpAddr::from(a)
                    }
                    _ => continue,
                };
                let max = if addr.is_ipv4() { 32 } else { 128 };
                out.push(format!("{addr}/{}", prefix.min(max)));
            }
            (3, 0) => reverse = g.val != 0,
            _ => {}
        }
    }
    Some((out, reverse))
}

// ---------------------------------------------------------------- detection

pub struct Detected {
    pub kind: &'static str,
    pub format: &'static str,
    pub categories: u32,
    pub tags: Option<Tags>,
}

pub fn detect(data: &[u8]) -> Result<Detected, String> {
    if data.len() >= 4 && &data[..3] == b"SRS" {
        return Ok(Detected { kind: RULESET, format: "binary", categories: 1, tags: None });
    }
    let text_start = data.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    if data.get(text_start) == Some(&b'{') {
        let v: Value = serde_json::from_slice(data).map_err(|_| "JSON не читается".to_string())?;
        if v["rules"].is_array() {
            return Ok(Detected { kind: RULESET, format: "source", categories: 1, tags: None });
        }
        return Err("это JSON, но не набор правил sing-box (нет rules)".into());
    }
    match scan_dat(data) {
        Ok((kind, tags)) => Ok(Detected { kind, format: "", categories: tags.len() as u32, tags: Some(tags) }),
        Err(_) => Err("это не geosite/geoip (.dat) и не набор правил sing-box (.srs, .json)".into()),
    }
}

const RULESET_FIELDS: &[&str] = &["domain", "domain_suffix", "domain_keyword", "domain_regex", "ip_cidr"];

fn ruleset_count(v: &Value) -> usize {
    v["rules"].as_array().into_iter().flatten()
        .map(|r| RULESET_FIELDS.iter().map(|k| match &r[*k] {
            Value::Array(a) => a.len(),
            Value::String(_) => 1,
            _ => 0,
        }).sum::<usize>()).sum()
}

fn str_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Value::String(s) => vec![s.clone()],
        _ => vec![],
    }
}

// ---------------------------------------------------------------- references

#[derive(Debug, PartialEq, Clone)]
pub enum Ref {
    /// `geosite:tag[@attr...]` (file None) or `ext-domain:file:tag`, `ext-site:file:tag`.
    Site { file: Option<String>, tag: String, attrs: Vec<String> },
    /// `geoip:tag`, `geoip:!tag`, `!geoip:tag` or `ext-ip:file:tag`.
    Ip { file: Option<String>, tag: String, invert: bool },
    /// `ext:file:tag`: domains or networks, whichever the file holds.
    Ext { file: String, tag: String, attrs: Vec<String>, invert: bool },
    /// `ruleset:name`: a sing-box rule-set file.
    RuleSet { name: String },
}

fn split_attrs(code: &str) -> (String, Vec<String>) {
    let mut parts = code.split('@');
    let tag = parts.next().unwrap_or("").trim().to_string();
    (tag, parts.map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).collect())
}

/// Reads one rule entry as a geo reference; None for plain domains, addresses and the rest.
pub fn parse_ref(raw: &str) -> Option<Ref> {
    let mut s = raw.trim();
    let mut invert = false;
    while let Some(rest) = s.strip_prefix('!') {
        invert = !invert;
        s = rest;
    }
    let lower = s.to_ascii_lowercase();
    let after = |p: &str| -> Option<&str> { if lower.starts_with(p) { Some(&s[p.len()..]) } else { None } };
    let code = |c: &str, invert: &mut bool| -> String {
        let mut c = c.trim();
        while let Some(rest) = c.strip_prefix('!') {
            *invert = !*invert;
            c = rest;
        }
        c.to_string()
    };
    if let Some(rest) = after("geosite:") {
        let (tag, attrs) = split_attrs(rest);
        return (!tag.is_empty()).then_some(Ref::Site { file: None, tag, attrs });
    }
    if let Some(rest) = after("geoip:") {
        let tag = code(rest, &mut invert);
        return (!tag.is_empty()).then_some(Ref::Ip { file: None, tag, invert });
    }
    if let Some(rest) = after("ruleset:") {
        let name = rest.trim().to_string();
        return (!name.is_empty()).then_some(Ref::RuleSet { name });
    }
    for (prefix, which) in [("ext-domain:", 1), ("ext-site:", 1), ("ext-ip:", 2), ("ext:", 0)] {
        let Some(rest) = after(prefix) else { continue };
        let (file, c) = rest.split_once(':')?;
        let file = file.trim().to_string();
        if file.is_empty() {
            return None;
        }
        let c = code(c, &mut invert);
        let (tag, attrs) = split_attrs(&c);
        if tag.is_empty() {
            return None;
        }
        return Some(match which {
            1 => Ref::Site { file: Some(file), tag, attrs },
            2 => Ref::Ip { file: Some(file), tag, invert },
            _ => Ref::Ext { file, tag, attrs, invert },
        });
    }
    None
}

/// Every geo ref anywhere in a config (values and object keys, such as DNS hosts).
pub fn refs_in(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if parse_ref(s).is_some() && !out.contains(s) {
                out.push(s.clone());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| refs_in(x, out)),
        Value::Object(o) => {
            for (k, x) in o {
                if parse_ref(k).is_some() && !out.contains(k) {
                    out.push(k.clone());
                }
                refs_in(x, out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- resolving

/// One usable file.
pub struct Avail {
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: String,
    pub path: PathBuf,
    pub sha: String,
    pub tags: Arc<Tags>,
    pub format: String,
    /// A rule-set's source form (for Xray, which cannot read rule-sets).
    pub ruleset: Option<Value>,
    /// The default geosite.dat / geoip.dat that plain `geosite:` / `geoip:` refs read.
    pub default: bool,
}

impl Avail {
    fn has(&self, tag: &str) -> bool {
        self.tags.contains_key(&tag.to_uppercase())
    }
    fn answers_to(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name) || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

/// What a config may use, fixed for the time one config is built.
pub struct Resolver {
    pub dir: PathBuf,
    singbox: Option<PathBuf>,
    pub files: Vec<Avail>,
    bytes: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    /// Shared with Geo: the categories converted so far, by file name (see Geo::reconvert).
    used: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    converting: Arc<Mutex<HashSet<String>>>,
    /// false: only rule-sets made before are used; the rest counts as pending (see rule_set).
    pub convert: AtomicBool,
}

/// What one entry of an Xray domain or IP list becomes.
#[derive(Default, Debug)]
struct Out {
    domains: Vec<String>,
    ips: Vec<String>,
    missing: bool,
}

impl Resolver {
    /// Nothing usable: every geo ref is dropped (and the shipped rule-sets of sing-box still work).
    pub fn empty() -> Resolver {
        Resolver { dir: PathBuf::new(), singbox: None, files: vec![], bytes: Mutex::new(HashMap::new()), used: Arc::default(),
                   converting: Arc::default(), convert: AtomicBool::new(true) }
    }

    fn by_name(&self, name: &str) -> Option<&Avail> {
        self.files.iter().find(|a| a.answers_to(name))
    }

    /// The file a plain `geosite:` / `geoip:` tag comes from: the default one first.
    fn with_tag(&self, kind: &str, tag: &str) -> Option<&Avail> {
        self.files.iter().filter(|a| a.kind == kind).find(|a| a.default && a.has(tag))
            .or_else(|| self.files.iter().filter(|a| a.kind == kind).find(|a| a.has(tag)))
    }

    fn data(&self, a: &Avail) -> Option<Arc<Vec<u8>>> {
        if let Some(d) = self.bytes.lock().unwrap().get(&a.name) {
            return Some(d.clone());
        }
        let d = Arc::new(std::fs::read(&a.path).ok()?);
        self.bytes.lock().unwrap().insert(a.name.clone(), d.clone());
        Some(d)
    }

    /// A ref → the file and tag it reads, with its kind. None when it cannot be resolved.
    fn locate(&self, r: &Ref) -> Option<(&Avail, String, Vec<String>, bool)> {
        let (a, tag, attrs, invert) = match r {
            Ref::Site { file, tag, attrs } => {
                let a = match file {
                    None => self.with_tag(SITE, tag)?,
                    Some(f) => self.by_name(f).filter(|a| a.kind == SITE && a.has(tag))?,
                };
                (a, tag, attrs.clone(), false)
            }
            Ref::Ip { file, tag, invert } => {
                let a = match file {
                    None => self.with_tag(IP, tag)?,
                    Some(f) => self.by_name(f).filter(|a| a.kind == IP && a.has(tag))?,
                };
                (a, tag, vec![], *invert)
            }
            Ref::Ext { file, tag, attrs, invert } => {
                let a = self.by_name(file).filter(|a| a.kind != RULESET && a.has(tag))?;
                (a, tag, if a.kind == SITE { attrs.clone() } else { vec![] }, *invert && a.kind == IP)
            }
            Ref::RuleSet { name } => {
                let a = self.files.iter().find(|a| a.kind == RULESET && (stem(&a.name).eq_ignore_ascii_case(name) || a.name.eq_ignore_ascii_case(name)))?;
                (a, &a.name, vec![], false)
            }
        };
        if a.kind == SITE && !attrs.is_empty() {
            // an attribute nothing in the category carries would leave an empty matcher
            let data = self.data(a)?;
            if site_items(&data, tag, &attrs).is_none_or(|d| d.is_empty()) {
                return None;
            }
        }
        Some((a, tag.clone(), attrs, invert))
    }

    /// The Xray form of one list entry.
    fn xray_entry(&self, raw: &str, ip_list: bool) -> Out {
        let Some(r) = parse_ref(raw) else {
            return if ip_list { Out { ips: vec![raw.to_string()], ..Default::default() } } else { Out { domains: vec![raw.to_string()], ..Default::default() } };
        };
        let missing = Out { missing: true, ..Default::default() };
        let Some((a, tag, attrs, invert)) = self.locate(&r) else { return missing };
        match a.kind.as_str() {
            SITE => {
                if ip_list {
                    return missing;
                }
                let keep = matches!(&r, Ref::Site { file: None, .. }) && a.default && a.name == "geosite.dat";
                let s = if keep {
                    raw.trim().to_string()
                } else {
                    format!("ext:{}:{}{}", a.name, tag, attrs.iter().map(|x| format!("@{x}")).collect::<String>())
                };
                Out { domains: vec![s], ..Default::default() }
            }
            IP => {
                let keep = matches!(&r, Ref::Ip { file: None, .. }) && a.default && a.name == "geoip.dat";
                let s = if keep { raw.trim().to_string() } else { format!("{}ext:{}:{}", if invert { "!" } else { "" }, a.name, tag) };
                Out { ips: vec![s], ..Default::default() }
            }
            _ => {
                let Some(j) = &a.ruleset else { return missing };
                let (domains, ips) = ruleset_for_xray(j);
                if domains.is_empty() && ips.is_empty() || (ip_list && ips.is_empty()) {
                    return missing;
                }
                Out { domains: if ip_list { vec![] } else { domains }, ips, missing: false }
            }
        }
    }

    /// Makes every geo ref of an Xray config point at a file that is really there; drops the
    /// rest. Returns the dropped refs.
    pub fn fix_xray(&self, cfg: &mut Value) -> Vec<String> {
        let mut missing: Vec<String> = vec![];
        if let Some(o) = cfg.as_object_mut() {
            o.remove("geodata"); // done by us (Geo::adopt_assets), into the same folder
        }
        if let Some(rules) = cfg.pointer_mut("/routing/rules").and_then(Value::as_array_mut) {
            let old = std::mem::take(rules);
            for rule in old {
                rules.extend(self.fix_rule(rule, &mut missing));
            }
        }
        if let Some(dns) = cfg.get_mut("dns").and_then(Value::as_object_mut) {
            if let Some(servers) = dns.get_mut("servers").and_then(Value::as_array_mut) {
                servers.retain_mut(|s| {
                    let Some(o) = s.as_object_mut() else { return true };
                    if let Some(had) = self.fix_key(o, "domains", false, &mut missing) {
                        if !had {
                            return false; // a server only for names that are gone: it would take every name
                        }
                    }
                    for k in ["expectIPs", "expectedIPs", "unexpectedIPs"] {
                        self.fix_key(o, k, true, &mut missing);
                    }
                    true
                });
            }
            if let Some(hosts) = dns.get_mut("hosts").and_then(Value::as_object_mut) {
                let keys: Vec<String> = hosts.keys().filter(|k| parse_ref(k).is_some()).cloned().collect();
                for k in keys {
                    let v = hosts.remove(&k).unwrap_or(Value::Null);
                    let out = self.xray_entry(&k, false);
                    match out.domains.as_slice() {
                        [one] if !out.missing => {
                            hosts.insert(one.clone(), v);
                        }
                        _ => missing.push(k),
                    }
                }
            }
        }
        // the rest: sniffing exclusions, freedom and finalmask address lists and the like
        if let Some(o) = cfg.as_object_mut() {
            for (k, v) in o.iter_mut() {
                if k != "routing" && k != "dns" {
                    self.fix_tree(v, &mut missing);
                }
            }
        }
        let mut seen = HashSet::new();
        missing.retain(|m| seen.insert(m.clone()));
        missing
    }

    /// Fixes the list under `key`. None: no such list. Some(false): it had entries and none is
    /// left. Some(true): something is left (or it was empty to begin with).
    fn fix_key(&self, o: &mut Map<String, Value>, key: &str, ip_list: bool, missing: &mut Vec<String>) -> Option<bool> {
        let list = str_list(o.get(key)?);
        if list.is_empty() || !list.iter().any(|x| parse_ref(x).is_some()) {
            return Some(true);
        }
        let mut keep = vec![];
        for item in &list {
            let out = self.xray_entry(item, ip_list);
            if out.missing {
                missing.push(item.clone());
            }
            keep.extend(if ip_list { out.ips } else { out.domains });
        }
        if keep.is_empty() {
            o.remove(key);
            return Some(false);
        }
        o.insert(key.into(), json!(keep));
        Some(true)
    }

    fn fix_tree(&self, v: &mut Value, missing: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                let keys: Vec<String> = o.keys().cloned().collect();
                for k in keys {
                    let is_list = o[&k].is_array() || o[&k].is_string();
                    if is_list && str_list(&o[&k]).iter().any(|x| parse_ref(x).is_some()) {
                        let ip_list = k.to_ascii_lowercase().contains("ip");
                        self.fix_key(o, &k, ip_list, missing);
                    } else if let Some(x) = o.get_mut(&k) {
                        self.fix_tree(x, missing);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.fix_tree(x, missing)),
            _ => {}
        }
    }

    /// One routing rule → zero, one or two rules. A rule loses a matcher list it had only when
    /// nothing in that list is left, and then it goes away whole: without the list it would
    /// match far more than the provider meant.
    fn fix_rule(&self, rule: Value, missing: &mut Vec<String>) -> Vec<Value> {
        let Value::Object(mut o) = rule else { return vec![rule] };
        let mut moved_ips: Vec<String> = vec![];
        let mut dead = false;
        for key in ["domain", "domains"] {
            let Some(v) = o.get(key) else { continue };
            let list = str_list(v);
            if list.is_empty() || !list.iter().any(|x| parse_ref(x).is_some()) {
                continue;
            }
            let mut keep = vec![];
            for item in &list {
                let out = self.xray_entry(item, false);
                if out.missing {
                    missing.push(item.clone());
                }
                keep.extend(out.domains);
                moved_ips.extend(out.ips);
            }
            if keep.is_empty() {
                o.remove(key);
                if moved_ips.is_empty() {
                    dead = true;
                }
            } else {
                o.insert(key.into(), json!(keep));
            }
        }
        for key in ["ip", "source", "sourceIP", "localIP"] {
            if self.fix_key(&mut o, key, true, missing) == Some(false) {
                dead = true;
            }
        }
        if dead {
            return vec![];
        }
        if moved_ips.is_empty() {
            return vec![Value::Object(o)];
        }
        // networks named in the domain list (geoip in exclusions, an ext: file with networks)
        let has_domains = o.contains_key("domain") || o.contains_key("domains");
        if o.contains_key("ip") {
            // the rule wants a domain and an address at once; there is no right place for these
            missing.extend(moved_ips);
            return if has_domains { vec![Value::Object(o)] } else { vec![] };
        }
        let mut ip_rule = o.clone();
        ip_rule.remove("domain");
        ip_rule.remove("domains");
        ip_rule.insert("ip".into(), json!(moved_ips));
        if has_domains {
            vec![Value::Object(o), Value::Object(ip_rule)]
        } else {
            vec![Value::Object(ip_rule)]
        }
    }

    /// A rule-set for sing-box standing for one ref: (tag, route.rule_set entry).
    pub fn singbox_set(&self, raw: &str) -> Option<(String, Value)> {
        self.rule_set(raw).ok()
    }

    /// Like `singbox_set`; the error says whether the ref resolves but its rule-set is not made
    /// yet (true: `convert` is off, or another connection is making it right now).
    pub fn rule_set(&self, raw: &str) -> Result<(String, Value), bool> {
        let r = parse_ref(raw).ok_or(false)?;
        let (a, tag, attrs, invert) = self.locate(&r).ok_or(false)?;
        if a.kind == RULESET {
            let set_tag = format!("ruleset-{}", stem(&a.name).to_lowercase());
            let format = if a.format == "binary" { "binary" } else { "source" };
            return Ok((set_tag, json!({"type": "local", "tag": "", "format": format, "path": a.path.to_string_lossy()})));
        }
        let suffix = format!("{}{}", attrs.iter().map(|x| format!("@{x}")).collect::<String>(), if invert { "-not" } else { "" });
        let tag_l = tag.to_lowercase();
        let set_tag = format!("{}-{}{}", stem(&a.name), tag_l, suffix);
        let key = format!("{}-{}-{}{}", a.sha.get(..12).unwrap_or("nohash"), a.kind, safe_name(&tag_l),
                          if suffix.is_empty() { String::new() } else { safe_name(&suffix) });
        let rules = self.dir.join("rules");
        let (srs, src) = (rules.join(format!("{key}.srs")), rules.join(format!("{key}.json")));
        let entry = |format: &str, path: &Path| json!({"type": "local", "tag": "", "format": format, "path": path.to_string_lossy()});
        // remembered, so that the next version of the file gets it converted in the background
        let canonical = if a.kind == SITE {
            format!("ext:{}:{}{}", a.name, tag, attrs.iter().map(|x| format!("@{x}")).collect::<String>())
        } else {
            format!("{}ext:{}:{}", if invert { "!" } else { "" }, a.name, tag)
        };
        self.used.lock().unwrap().entry(a.name.clone()).or_default().insert(canonical);
        let busy = self.converting.lock().unwrap().contains(&key);
        if !busy {
            if srs.is_file() {
                return Ok((set_tag, entry("binary", &srs)));
            }
            if src.is_file() {
                return Ok((set_tag, entry("source", &src)));
            }
        }
        if busy || !self.convert.load(Ordering::SeqCst) || !self.converting.lock().unwrap().insert(key.clone()) {
            return Err(true);
        }
        let made = self.make_rule_set(a, &tag, &attrs, invert, &rules, &srs, &src);
        self.converting.lock().unwrap().remove(&key);
        made.map(|(format, path)| (set_tag, entry(format, &path))).ok_or(false)
    }

    /// Writes the source rule-set and compiles it when sing-box is at hand. Compiling is what
    /// takes the time (sing-box 1.14, optimised build: ru-blocked with 74k domains about 0.9 s,
    /// category-ads-all with 183k 2.4 s, ru-blocked-all with 1.6M 17 s); reading and decoding
    /// the 90 MB file is 0.1 s.
    #[allow(clippy::too_many_arguments)]
    fn make_rule_set(&self, a: &Avail, tag: &str, attrs: &[String], invert: bool, rules: &Path, srs: &Path, src: &Path)
                     -> Option<(&'static str, PathBuf)> {
        let data = self.data(a)?;
        let body = if a.kind == SITE {
            site_ruleset(&site_items(&data, tag, attrs)?)?
        } else {
            let (cidrs, reverse) = ip_items(&data, tag)?;
            ip_ruleset(&cidrs, invert != reverse)?
        };
        let _ = std::fs::create_dir_all(rules);
        write_atomic(src, &serde_json::to_vec(&body).ok()?).ok()?;
        if let Some(bin) = &self.singbox {
            let tmp = srs.with_file_name(format!("{}.part", srs.file_name()?.to_string_lossy()));
            if run(bin, &["rule-set", "compile", "--output", &tmp.to_string_lossy(), &src.to_string_lossy()]) && std::fs::rename(&tmp, srs).is_ok() {
                let _ = std::fs::remove_file(src);
                return Some(("binary", srs.to_path_buf()));
            }
            let _ = std::fs::remove_file(&tmp);
        }
        Some(("source", src.to_path_buf()))
    }
}

/// A geosite category as a sing-box source rule-set (None when it is empty).
pub fn site_ruleset(items: &[SiteItem]) -> Option<Value> {
    let mut m: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (kind, value) in items {
        let key = match kind {
            0 => "domain_keyword",
            1 => "domain_regex",
            2 => "domain_suffix",
            3 => "domain",
            _ => continue,
        };
        m.entry(key).or_default().push(value);
    }
    if m.is_empty() {
        return None;
    }
    Some(json!({"version": 2, "rules": [m]}))
}

/// A geoip category as a sing-box source rule-set.
pub fn ip_ruleset(cidrs: &[String], invert: bool) -> Option<Value> {
    if cidrs.is_empty() {
        return None;
    }
    let mut rule = json!({"ip_cidr": cidrs});
    if invert {
        rule["invert"] = json!(true);
    }
    Some(json!({"version": 2, "rules": [rule]}))
}

/// A sing-box rule-set's domains and networks in Xray's spelling. Logical and inverted rules
/// have no Xray form and are skipped.
pub fn ruleset_for_xray(v: &Value) -> (Vec<String>, Vec<String>) {
    let (mut d, mut ip) = (vec![], vec![]);
    for r in v["rules"].as_array().into_iter().flatten() {
        if r["type"].as_str() == Some("logical") || r["invert"].as_bool() == Some(true) {
            continue;
        }
        d.extend(str_list(&r["domain"]).into_iter().map(|x| format!("full:{x}")));
        d.extend(str_list(&r["domain_suffix"]).into_iter().map(|x| format!("domain:{}", x.trim_start_matches('.'))));
        d.extend(str_list(&r["domain_keyword"]).into_iter().map(|x| format!("keyword:{x}")));
        d.extend(str_list(&r["domain_regex"]).into_iter().map(|x| format!("regexp:{x}")));
        ip.extend(str_list(&r["ip_cidr"]));
    }
    (d, ip)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    // a tiny protobuf writer for test files
    fn put_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    fn put_bytes(out: &mut Vec<u8>, field: u64, data: &[u8]) {
        put_varint(out, field << 3 | 2);
        put_varint(out, data.len() as u64);
        out.extend_from_slice(data);
    }
    fn put_uint(out: &mut Vec<u8>, field: u64, v: u64) {
        put_varint(out, field << 3);
        put_varint(out, v);
    }

    /// A GeoSiteList: (code, [(type, value, attrs)]).
    pub fn geosite(entries: &[(&str, &[(u64, &str, &[&str])])]) -> Vec<u8> {
        let mut out = vec![];
        for (code, domains) in entries {
            let mut e = vec![];
            put_bytes(&mut e, 1, code.as_bytes());
            for (t, v, attrs) in domains.iter() {
                let mut d = vec![];
                if *t != 0 {
                    put_uint(&mut d, 1, *t);
                }
                put_bytes(&mut d, 2, v.as_bytes());
                for a in attrs.iter() {
                    let mut at = vec![];
                    put_bytes(&mut at, 1, a.as_bytes());
                    put_uint(&mut at, 2, 1);
                    put_bytes(&mut d, 3, &at);
                }
                put_bytes(&mut e, 2, &d);
            }
            put_bytes(&mut out, 1, &e);
        }
        out
    }

    /// A GeoIPList: (code, [(ip bytes, prefix)]).
    pub fn geoip(entries: &[(&str, &[(&[u8], u64)])]) -> Vec<u8> {
        let mut out = vec![];
        for (code, cidrs) in entries {
            let mut e = vec![];
            put_bytes(&mut e, 1, code.as_bytes());
            for (ip, prefix) in cidrs.iter() {
                let mut c = vec![];
                put_bytes(&mut c, 1, ip);
                put_uint(&mut c, 2, *prefix);
                put_bytes(&mut e, 2, &c);
            }
            put_bytes(&mut out, 1, &e);
        }
        out
    }

    pub fn sample_site() -> Vec<u8> {
        geosite(&[
            ("CATEGORY-RU", &[(2, "yandex.ru", &[]), (3, "vk.com", &[]), (0, "sber", &[]), (1, "^mail\\.ru$", &[])]),
            ("GOOGLE", &[(2, "google.com", &["ads"]), (2, "youtube.com", &[])]),
        ])
    }

    pub fn sample_ru_site() -> Vec<u8> {
        geosite(&[("RU-BLOCKED", &[(2, "blocked.example", &[]), (3, "x.blocked.example", &[])]), ("CATEGORY-RU", &[(2, "ya.ru", &[])])])
    }

    pub fn sample_ip() -> Vec<u8> {
        geoip(&[("RU", &[(&[5, 8, 0, 0], 16), (&[0x2a, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 12)]), ("PRIVATE", &[(&[10, 0, 0, 0], 8)])])
    }

    pub fn sample_ru_ip() -> Vec<u8> {
        geoip(&[("RU-BLOCKED", &[(&[1, 2, 3, 0], 24)])])
    }

    #[test]
    fn protobuf_lists_tags_and_tells_the_kinds_apart() {
        let (kind, tags) = scan_dat(&sample_site()).unwrap();
        assert_eq!(kind, SITE);
        assert_eq!(tags.get("CATEGORY-RU"), Some(&4));
        assert_eq!(tags.get("GOOGLE"), Some(&2));
        let (kind, tags) = scan_dat(&sample_ip()).unwrap();
        assert_eq!(kind, IP);
        assert_eq!(tags.get("RU"), Some(&2));
        assert!(scan_dat(b"hello world, not protobuf").is_err());
        assert!(scan_dat(&[]).is_err());
        // a domain whose type is the default (keyword) has no type field at all: still geosite
        let only_plain = geosite(&[("X", &[(0, "abcd", &[])])]);
        assert_eq!(scan_dat(&only_plain).unwrap().0, SITE);
        assert_eq!(detect(&sample_ip()).unwrap().kind, IP);
        assert_eq!(detect(b"SRS\x03....").unwrap().kind, RULESET);
        assert_eq!(detect(br#" {"version": 2, "rules": [{"domain": ["a.b"]}]}"#).unwrap().format, "source");
        assert!(detect(br#"{"log": {}}"#).is_err());
    }

    #[test]
    fn protobuf_categories_decode() {
        let items = site_items(&sample_site(), "category-ru", &[]).unwrap();
        assert_eq!(items, vec![(2, "yandex.ru".into()), (3, "vk.com".into()), (0, "sber".into()), (1, "^mail\\.ru$".into())]);
        assert_eq!(site_items(&sample_site(), "GOOGLE", &["ads".into()]).unwrap(), vec![(2, "google.com".into())]);
        assert!(site_items(&sample_site(), "nope", &[]).is_none());
        let (cidrs, rev) = ip_items(&sample_ip(), "ru").unwrap();
        assert_eq!(cidrs, vec!["5.8.0.0/16".to_string(), "2a00::/12".to_string()]);
        assert!(!rev);
        let set = site_ruleset(&items).unwrap();
        assert_eq!(set["rules"][0]["domain_suffix"], json!(["yandex.ru"]));
        assert_eq!(set["rules"][0]["domain"], json!(["vk.com"]));
        assert_eq!(set["rules"][0]["domain_keyword"], json!(["sber"]));
        assert_eq!(set["rules"][0]["domain_regex"], json!(["^mail\\.ru$"]));
        assert_eq!(ip_ruleset(&cidrs, true).unwrap()["rules"][0]["invert"], true);
    }

    #[test]
    fn refs_parse() {
        assert_eq!(parse_ref("geosite:google@ads"), Some(Ref::Site { file: None, tag: "google".into(), attrs: vec!["ads".into()] }));
        assert_eq!(parse_ref("geoip:!ru"), Some(Ref::Ip { file: None, tag: "ru".into(), invert: true }));
        assert_eq!(parse_ref("!geoip:ru"), Some(Ref::Ip { file: None, tag: "ru".into(), invert: true }));
        assert_eq!(parse_ref("ext:geosite_RU.dat:ru-blocked"), Some(Ref::Ext { file: "geosite_RU.dat".into(), tag: "ru-blocked".into(), attrs: vec![], invert: false }));
        assert_eq!(parse_ref("ext-ip:zkeenip.dat:ru"), Some(Ref::Ip { file: Some("zkeenip.dat".into()), tag: "ru".into(), invert: false }));
        assert_eq!(parse_ref("ruleset:my-list"), Some(Ref::RuleSet { name: "my-list".into() }));
        assert_eq!(parse_ref("domain:example.org"), None);
        assert_eq!(parse_ref("1.2.3.0/24"), None);
        assert_eq!(parse_ref("ext::x"), None);
        assert_eq!(parse_ref("geosite:"), None);
    }

    /// A geo folder with the default files, the runetfreedom pair and one user rule-set.
    pub fn sample_dir(name: &str) -> (PathBuf, Arc<Geo>) {
        sample_dir_with(name, None)
    }

    pub fn sample_dir_with(name: &str, singbox: Option<PathBuf>) -> (PathBuf, Arc<Geo>) {
        let root = std::env::temp_dir().join(format!("quadra-geo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let shipped = root.join("shipped");
        std::fs::create_dir_all(&shipped).unwrap();
        std::fs::write(shipped.join("geosite.dat"), sample_site()).unwrap();
        std::fs::write(shipped.join("geoip.dat"), sample_ip()).unwrap();
        let geo = Arc::new(Geo::open(root.join("geo"), shipped, singbox));
        geo.sync_defaults();
        for (id, data) in [("ru-geosite", sample_ru_site()), ("ru-geoip", sample_ru_ip())] {
            geo.store(Some(id), "", data, |_, _| Entry::default(), String::new(), String::new()).unwrap();
        }
        let rs = root.join("mine.json");
        std::fs::write(&rs, br#"{"version": 2, "rules": [{"domain_suffix": [".rs.example"], "ip_cidr": ["9.9.9.0/24"]}]}"#).unwrap();
        geo.add_file(&rs).unwrap();
        (root, geo)
    }

    #[test]
    fn files_are_added_listed_and_removed() {
        let (root, geo) = sample_dir("files");
        let list = geo.list();
        let site = list.iter().find(|f| f["id"] == "default-geosite").unwrap();
        assert_eq!(site["categories"], 2);
        assert!(site["path"].as_str().unwrap().ends_with("geosite.dat"));
        let ru = list.iter().find(|f| f["id"] == "ru-geosite").unwrap();
        assert_eq!(ru["name"], "geosite_RU.dat");
        let mine = list.iter().find(|f| f["source"] == "file").unwrap();
        assert_eq!(mine["kind"], "ruleset");
        assert_eq!(mine["name"], "mine.json");
        // the same name again gets a suffix; a .dat named like a built-in too
        let again = geo.add_file(&root.join("mine.json")).unwrap();
        assert_eq!(again["name"], "mine-2.json");
        let user_site = root.join("geosite.dat");
        std::fs::write(&user_site, sample_ru_site()).unwrap();
        let v = geo.add_file(&user_site).unwrap();
        assert_eq!(v["name"], "geosite-2.dat");
        assert_eq!(v["kind"], "geosite");
        let cats = geo.categories(v["id"].as_str().unwrap(), Some("BLOCK"), None).unwrap();
        assert_eq!(cats, vec![json!({"tag": "ru-blocked", "count": 2})]);
        assert_eq!(geo.categories(mine["id"].as_str().unwrap(), None, None).unwrap()[0]["count"], 2);
        assert!(geo.remove("default-geosite").is_err());
        geo.remove(v["id"].as_str().unwrap()).unwrap();
        assert!(!geo.dir.join("geosite-2.dat").exists());
        assert_eq!(geo.set_enabled("ru-geoip", false).unwrap()["enabled"], false);
        // a broken file is refused
        let junk = root.join("junk.dat");
        std::fs::write(&junk, b"definitely not geodata").unwrap();
        assert!(geo.add_file(&junk).is_err());
        // reopening keeps everything
        let again = Geo::open(geo.dir.clone(), root.join("shipped"), None);
        assert_eq!(again.list().len(), geo.list().len());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ext_refs_resolve_to_files_that_are_there() {
        let (root, geo) = sample_dir("ext");
        let r = geo.resolver();
        // the default file has it: unchanged
        assert_eq!(r.xray_entry("geosite:category-ru", false).domains, vec!["geosite:category-ru"]);
        // only the runetfreedom file has it: an ext: ref to that file
        assert_eq!(r.xray_entry("geosite:ru-blocked", false).domains, vec!["ext:geosite_RU.dat:ru-blocked"]);
        // an alias of a built-in
        assert_eq!(r.xray_entry("ext:geosite-ru.dat:ru-blocked", false).domains, vec!["ext:geosite_RU.dat:ru-blocked"]);
        assert_eq!(r.xray_entry("ext:geoip_RU.dat:ru-blocked", true).ips, vec!["ext:geoip_RU.dat:ru-blocked"]);
        assert_eq!(r.xray_entry("geoip:!ru", true).ips, vec!["geoip:!ru"]);
        assert_eq!(r.xray_entry("geoip:ru-blocked", true).ips, vec!["ext:geoip_RU.dat:ru-blocked"]);
        // attributes: one that exists, one that does not
        assert_eq!(r.xray_entry("geosite:google@ads", false).domains, vec!["geosite:google@ads"]);
        assert!(r.xray_entry("geosite:google@cn", false).missing);
        // nowhere at all
        assert!(r.xray_entry("geosite:no-such-tag", false).missing);
        assert!(r.xray_entry("ext:unknown.dat:x", false).missing);
        assert!(r.xray_entry("ext:geosite_RU.dat:no-such-tag", false).missing);
        // geoip named among domains moves to the address list
        let out = r.xray_entry("geoip:ru", false);
        assert_eq!((out.domains.len(), out.ips.clone()), (0, vec!["geoip:ru".to_string()]));
        // a rule-set
        let out = r.xray_entry("ruleset:mine", false);
        assert_eq!(out.domains, vec!["domain:rs.example"]);
        assert_eq!(out.ips, vec!["9.9.9.0/24"]);
        // plain entries pass through
        assert_eq!(r.xray_entry("domain:example.org", false).domains, vec!["domain:example.org"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_xray_config_full_of_refs_keeps_what_resolves() {
        let (root, geo) = sample_dir("xray");
        let r = geo.resolver();
        let mut cfg = json!({
            "geodata": {"assets": [{"url": "https://example.org/x.dat", "file": "x.dat"}]},
            "inbounds": [{"tag": "socks", "protocol": "socks", "sniffing": {"enabled": true, "domainsExcluded": ["geosite:nope", "courier.push.apple.com"]}}],
            "dns": {"hosts": {"geosite:category-ru": "1.1.1.1", "geosite:bogus": "2.2.2.2", "plain.example": "3.3.3.3"},
                    "servers": [{"address": "77.88.8.8", "domains": ["geosite:bogus"]},
                                {"address": "1.1.1.1", "domains": ["geosite:ru-blocked"], "expectIPs": ["geoip:bogus"]}, "8.8.8.8"]},
            "routing": {"rules": [
                {"type": "field", "domain": ["geosite:category-ru", "ext:geosite_RU.dat:ru-blocked", "geosite:bogus"], "outboundTag": "direct"},
                {"type": "field", "domain": ["geosite:bogus", "ext:zkeen.dat:bogus"], "outboundTag": "block"},
                {"type": "field", "ip": ["geoip:private", "geoip:ru", "ext:geoip_RU.dat:ru-blocked", "!geoip:bogus"], "outboundTag": "direct"},
                {"type": "field", "ip": ["ext:nothing.dat:x"], "outboundTag": "direct"},
                {"type": "field", "domain": ["geoip:ru", "geosite:google"], "outboundTag": "direct"},
                {"type": "field", "inboundTag": ["socks"], "outboundTag": "proxy"}]}});
        let missing = r.fix_xray(&mut cfg);
        assert!(cfg.get("geodata").is_none());
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 5, "{rules:#?}");
        assert_eq!(rules[0]["domain"], json!(["geosite:category-ru", "ext:geosite_RU.dat:ru-blocked"]));
        assert_eq!(rules[1]["ip"], json!(["geoip:private", "geoip:ru", "ext:geoip_RU.dat:ru-blocked"]));
        assert_eq!(rules[2]["domain"], json!(["geosite:google"]));
        assert_eq!(rules[3]["ip"], json!(["geoip:ru"]));
        assert!(rules[3].get("domain").is_none());
        assert_eq!(rules[4]["inboundTag"], json!(["socks"]));
        let servers = cfg["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0]["domains"], json!(["ext:geosite_RU.dat:ru-blocked"]));
        assert!(servers[0].get("expectIPs").is_none());
        let hosts = cfg["dns"]["hosts"].as_object().unwrap();
        assert!(hosts.contains_key("geosite:category-ru") && hosts.contains_key("plain.example") && hosts.len() == 2);
        assert_eq!(cfg["inbounds"][0]["sniffing"]["domainsExcluded"], json!(["courier.push.apple.com"]));
        for m in ["geosite:bogus", "ext:zkeen.dat:bogus", "!geoip:bogus", "ext:nothing.dat:x", "geoip:bogus", "geosite:nope"] {
            assert!(missing.contains(&m.to_string()), "{m} not in {missing:?}");
        }
        assert_eq!(missing.iter().filter(|m| *m == "geosite:bogus").count(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn categories_become_rule_sets_once() {
        let (root, geo) = sample_dir("sets");
        // not made yet and no making allowed: pending, unlike a ref nothing resolves
        let frozen = geo.resolver();
        frozen.convert.store(false, Ordering::SeqCst);
        assert_eq!(frozen.rule_set("geosite:ru-blocked"), Err(true));
        assert_eq!(frozen.rule_set("geosite:bogus"), Err(false));
        let r = geo.resolver();
        let (tag, entry) = r.singbox_set("geosite:ru-blocked").unwrap();
        assert_eq!(frozen.rule_set("geosite:ru-blocked").unwrap().1["path"], entry["path"]);
        assert_eq!(tag, "geosite_RU-ru-blocked");
        assert_eq!(entry["format"], "source"); // no sing-box binary in this test
        let path = PathBuf::from(entry["path"].as_str().unwrap());
        let body: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(body["rules"][0]["domain_suffix"], json!(["blocked.example"]));
        assert_eq!(body["rules"][0]["domain"], json!(["x.blocked.example"]));
        // the second time it comes from the cache
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(r.singbox_set("geosite:ru-blocked").unwrap().1["path"], entry["path"]);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
        let (tag, entry) = r.singbox_set("geoip:!ru").unwrap();
        assert_eq!(tag, "geoip-ru-not");
        let body: Value = serde_json::from_slice(&std::fs::read(entry["path"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(body["rules"][0]["invert"], true);
        assert_eq!(r.singbox_set("ruleset:mine").unwrap().0, "ruleset-mine");
        assert!(r.singbox_set("geosite:bogus").is_none());
        assert!(r.singbox_set("ext:zkeen.dat:x").is_none());
        // new content for a file drops its old conversions, and the ones in use are made again
        let newer = geosite(&[("RU-BLOCKED", &[(2, "blocked.example", &[]), (2, "more.example", &[])])]);
        geo.store(Some("ru-geosite"), "", newer, |_, _| Entry::default(), String::new(), String::new()).unwrap();
        assert!(!path.exists());
        geo.reconvert("geosite_RU.dat");
        let sha = geo.entry("ru-geosite").unwrap().sha;
        let remade: Vec<String> = std::fs::read_dir(geo.dir.join("rules")).unwrap().flatten()
            .map(|f| f.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(&sha[..12])).collect();
        assert_eq!(remade, vec![format!("{}-geosite-ru-blocked.json", &sha[..12])]);
        // and that list survives a restart
        geo.save();
        let again = Geo::open(geo.dir.clone(), root.join("shipped"), None);
        assert!(again.used.lock().unwrap()["geosite_RU.dat"].contains("ext:geosite_RU.dat:ru-blocked"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn known_names_are_fetched_and_unknown_ones_are_not() {
        let root = std::env::temp_dir().join(format!("quadra-geo-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("shipped")).unwrap();
        std::fs::write(root.join("shipped").join("geosite.dat"), sample_site()).unwrap();
        let geo = Geo::open(root.join("geo"), root.join("shipped"), None);
        let plan = geo.plan_fetch(&["ext:geosite_ru.dat:ru-blocked".into(), "ext:whatever.dat:x".into(), "geosite:category-ru".into()]);
        assert_eq!(plan, vec!["ru-geosite"]);
        // a plain tag the default file lacks: the set known to carry it is fetched
        let plan = geo.plan_fetch(&["geoip:ru-blocked".into(), "geosite:whitelist".into(), "geosite:never-heard-of".into()]);
        assert_eq!(plan, vec!["ru-geoip", "roscomvpn-geosite"]);
        // a provider's own asset list makes a name known
        geo.adopt_assets(&json!({"geodata": {"assets": [{"url": "https://example.org/lists.dat.gz", "file": "provider-lists.dat"},
                                                        {"url": "https://example.org/z.dat", "file": "zkeen.dat"}]}}));
        let plan = geo.plan_fetch(&["ext:provider-lists.dat:youtube".into(), "ext:zkeen.dat:youtube".into()]);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[1], "zkeen");
        assert!(geo.list().iter().any(|f| f["name"] == "provider-lists.dat" && f["source"] == "url"));
        assert!(!geo.list().iter().any(|f| f["url"] == "https://example.org/z.dat"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Starts a core for a few seconds; true when it says it is up.
    fn starts(bin: &std::ffi::OsStr, args: &[&str], assets: &Path, needle: &str) -> (bool, String) {
        use std::io::Read;
        let log = std::env::temp_dir().join(format!("quadra-core-{}-{}.log", std::process::id(), needle.len()));
        let out = std::fs::File::create(&log).unwrap();
        let mut child = std::process::Command::new(bin).args(args).env("XRAY_LOCATION_ASSET", assets)
            .stdout(out.try_clone().unwrap()).stderr(out).spawn().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut text = String::new();
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
            text.clear();
            let _ = std::fs::File::open(&log).unwrap().read_to_string(&mut text);
            if text.contains(needle) || child.try_wait().ok().flatten().is_some() {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::File::open(&log).unwrap().read_to_string(&mut text);
        (text.contains(needle), text)
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    /// The 0.4.0 crash, end to end with the real cores: a provider config with refs nobody has
    /// stops Xray as it is, and starts both cores once it went through the geo data.
    /// Runs when QUADRA_TEST_XRAY and/or QUADRA_TEST_SINGBOX name the binaries.
    #[test]
    fn cores_start_with_refs_nobody_has() {
        let xray = std::env::var_os("QUADRA_TEST_XRAY");
        let singbox = std::env::var_os("QUADRA_TEST_SINGBOX");
        if xray.is_none() && singbox.is_none() {
            eprintln!("skipped: QUADRA_TEST_XRAY / QUADRA_TEST_SINGBOX not set");
            return;
        }
        let (root, geo) = sample_dir_with("cores", singbox.as_ref().map(PathBuf::from));
        let provider = json!({
            "remarks": "bogus refs",
            "inbounds": [{"tag": "socks", "port": 10808, "listen": "127.0.0.1", "protocol": "socks", "settings": {"udp": true}}],
            "outbounds": [
                {"tag": "proxy", "protocol": "vless", "settings": {"vnext": [{"address": "127.0.0.1", "port": 9, "users": [{"id": "11111111-2222-3333-4444-555555555555", "encryption": "none"}]}]}},
                {"tag": "direct", "protocol": "freedom"}, {"tag": "block", "protocol": "blackhole"}],
            "dns": {"servers": [{"address": "1.1.1.1", "domains": ["geosite:no-such-category"]}, "8.8.8.8"]},
            "routing": {"domainStrategy": "AsIs", "rules": [
                {"type": "field", "domain": ["geosite:category-ru", "geosite:ru-blocked", "geosite:no-such-category", "ext:geosite_RU.dat:ru-blocked"], "outboundTag": "direct"},
                {"type": "field", "domain": ["ext:missing-file.dat:ads", "geosite:category-ads-all"], "outboundTag": "block"},
                {"type": "field", "ip": ["geoip:private", "geoip:ru", "ext:geoip_RU.dat:ru-blocked", "geoip:no-such-country", "ext:zkeenip.dat:youtube"], "outboundTag": "direct"},
                {"type": "field", "inboundTag": ["socks"], "outboundTag": "proxy"}]}});
        let server = crate::model::Server { config: Some(provider), ..Default::default() };
        let settings = crate::model::Settings {
            socks_port: free_port(), http_port: free_port(), core: "auto".into(),
            bypass_domains: vec!["geosite:ru-available-only-inside".into(), "ext:nothing.dat:x".into(), "geoip:ru-blocked".into(), "ruleset:mine".into(), "example.org".into()],
            ..Default::default()
        };
        if let Some(bin) = &xray {
            let mut cfg = crate::config::build(&server, &settings, false, free_port()).unwrap();
            let raw = root.join("xray-raw.json");
            std::fs::write(&raw, cfg.to_string()).unwrap();
            let (ok, text) = starts(bin, &["run", "-c", &raw.to_string_lossy()], &geo.dir, "started");
            eprintln!("xray, raw config: {}", text.lines().find(|l| l.contains("geodata")).unwrap_or("(no geodata line)"));
            assert!(!ok && text.contains("geodata"), "the raw config should stop Xray on geodata:\n{text}");
            let missing = geo.resolver().fix_xray(&mut cfg);
            assert!(missing.contains(&"geosite:no-such-category".to_string()) && missing.contains(&"ext:missing-file.dat:ads".to_string()));
            let fixed = root.join("xray-fixed.json");
            std::fs::write(&fixed, cfg.to_string()).unwrap();
            let (ok, text) = starts(bin, &["run", "-c", &fixed.to_string_lossy()], &geo.dir, "started");
            eprintln!("xray, fixed config: {}", text.lines().find(|l| l.contains("started")).unwrap_or(""));
            assert!(ok, "Xray did not start:\n{text}");
        }
        if let Some(bin) = &singbox {
            let resolver = geo.resolver();
            let rules = root.join("no-shipped-rules");
            let local = crate::singbox::Local { tun: false, clash_port: free_port(), probe_port: free_port(), secret: "s", rules_dir: &rules, geo: Some(&resolver) };
            let b = crate::singbox::build(&server, &settings, &local).unwrap();
            assert!(b.missing.contains(&"geosite:no-such-category".to_string()));
            let sets = b.config["route"]["rule_set"].as_array().unwrap();
            assert!(sets.iter().any(|s| s["format"] == "binary"), "rule-sets were not compiled: {sets:?}");
            let path = root.join("singbox.json");
            std::fs::write(&path, b.config.to_string()).unwrap();
            let (ok, text) = starts(bin, &["run", "--disable-color", "-c", &path.to_string_lossy()], &geo.dir, "sing-box started");
            eprintln!("sing-box: {} ({} rule-sets)", text.lines().find(|l| l.contains("started")).unwrap_or(""), sets.len());
            assert!(ok, "sing-box did not start:\n{text}\n{}", b.config);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// Timing on real files: QUADRA_GEO_REAL=<folder with geosite_RU.dat and geoip_RU.dat>,
    /// QUADRA_TEST_SINGBOX for compiling. `cargo test --release timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn timing_on_real_files() {
        let Some(dir) = std::env::var_os("QUADRA_GEO_REAL").map(PathBuf::from) else { return };
        let t = std::time::Instant::now();
        let site = std::fs::read(dir.join("geosite_RU.dat")).unwrap();
        eprintln!("read geosite_RU.dat ({} MB): {:?}", site.len() >> 20, t.elapsed());
        let t = std::time::Instant::now();
        let (kind, tags) = scan_dat(&site).unwrap();
        eprintln!("scan: {kind}, {} tags, {:?}", tags.len(), t.elapsed());
        for tag in ["ru-blocked", "ru-blocked-all", "category-ads-all", "ru-available-only-inside"] {
            let t = std::time::Instant::now();
            let items = site_items(&site, tag, &[]).unwrap();
            let body = serde_json::to_vec(&site_ruleset(&items).unwrap()).unwrap();
            let decoded = t.elapsed();
            let src = std::env::temp_dir().join(format!("quadra-timing-{tag}.json"));
            let srs = src.with_extension("srs");
            std::fs::write(&src, &body).unwrap();
            let t = std::time::Instant::now();
            let compiled = std::env::var_os("QUADRA_TEST_SINGBOX")
                .map(|b| run(Path::new(&b), &["rule-set", "compile", "--output", &srs.to_string_lossy(), &src.to_string_lossy()]));
            eprintln!("{tag}: {} domains, json {} KB, decode {:?}, compile {:?} ({compiled:?}, srs {} KB)",
                items.len(), body.len() >> 10, decoded, t.elapsed(), std::fs::metadata(&srs).map(|m| m.len() >> 10).unwrap_or(0));
        }
        let ip = std::fs::read(dir.join("geoip_RU.dat")).unwrap();
        let t = std::time::Instant::now();
        let (cidrs, _) = ip_items(&ip, "ru").unwrap();
        eprintln!("geoip ru: {} networks, {:?}", cidrs.len(), t.elapsed());
    }

    #[test]
    fn rule_sets_read_back_for_xray() {
        let j = json!({"version": 2, "rules": [{"domain": ["a.b"], "domain_suffix": [".c.d"], "domain_keyword": ["k"], "domain_regex": ["^r$"], "ip_cidr": ["1.1.1.0/24"]},
            {"type": "logical", "mode": "and", "rules": []}, {"domain": ["skip.me"], "invert": true}]});
        let (d, ip) = ruleset_for_xray(&j);
        assert_eq!(d, vec!["full:a.b", "domain:c.d", "keyword:k", "regexp:^r$"]);
        assert_eq!(ip, vec!["1.1.1.0/24"]);
        assert_eq!(ruleset_count(&j), 6);
    }
}
