import { invoke, listen, pickFile } from './api.js';
import { fixable, many } from './geo.js';

export const app = $state({
  ready: false,
  settings: null,
  subs: [],
  servers: [],
  status: { state: 'off', server: null, since: 0, ms: null, error: null, exit: null, tun: false },
  hwid: '',
  admin: false,
  elevated: false,
  platform: 'windows',
  tiling: false,
  relaunchForTun: false,
  version: '',
  userAgent: '',
  traffic: { up: 0, down: 0, tup: 0, tdown: 0 },
  history: [],
  panel: null, // 'routes' | 'log' | 'settings'
  popup: null, // { kind, ... }
  menu: null, // context menu { x, y, items }
  toasts: [],
  query: '',
  pinging: false,
  busy: {},
  pickHome: false,
  apps: null, // programs to choose from in the exclusions, loaded when first needed
  geo: null, // GeoFile[] (docs/geo-api.md), loaded when a screen needs it
  geoError: '', // why the list could not be read
  geoMissing: null, // { server, refs, fixed }: geo refs the last connection had to drop
  geoFocus: false, // the settings panel scrolls to its geo data section when it opens
  now: Date.now(),
  logTick: 0,
});

/** Core output. Kept outside the reactive state: thousands of lines, read only by the log panel. */
export const logLines = [];

const HISTORY = 48;
let toastId = 0;

export function toast(text, kind = 'info') {
  const id = ++toastId;
  app.toasts.push({ id, text, kind });
  setTimeout(() => {
    app.toasts = app.toasts.filter((t) => t.id !== id);
  }, kind === 'error' ? 6500 : 3600);
}

const errText = (e) => (typeof e === 'string' ? e : (e?.message ?? 'что-то пошло не так'));
const fail = (e) => toast(errText(e), 'error');

let saveTimer;
export function save() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => invoke('save_settings', { settings: $state.snapshot(app.settings) }).catch(fail), 220);
}

export function set(key, value) {
  app.settings[key] = value;
  save();
}

function applyData(d, first = false) {
  app.subs = d.subs;
  app.servers = d.servers;
  if (first) {
    app.settings = d.settings;
    app.status = d.status;
    app.hwid = d.hwid;
    app.admin = d.admin;
    app.elevated = d.elevated;
    app.platform = d.platform ?? 'windows';
    app.tiling = !!d.tiling;
    app.relaunchForTun = !!d.relaunchForTun;
    // a phone has one mode: the system VPN
    if (app.platform === 'android') app.settings.mode = 'tun';
    app.settings.bypass_mode ??= 'exclude';
    app.settings.geo_auto_update ??= true;
    app.version = d.version;
    app.userAgent = d.userAgent;
  } else {
    app.settings.home = d.settings.home;
  }
}

export const serverById = (id) => app.servers.find((s) => s.id === id);

export function bestServer() {
  let best = null;
  for (const s of app.servers) if (s.ping > 0 && (!best || s.ping < best.ping)) best = s;
  return best ?? app.servers[0] ?? null;
}

/** The server a click on "connect" will use right now. */
export function target() {
  const sel = app.settings?.selected;
  if (sel && sel !== 'auto') return serverById(sel) ?? null;
  return bestServer();
}

export async function pingAll(ids = []) {
  if (app.pinging || !app.servers.length) return;
  app.pinging = true;
  try {
    await invoke('ping', { ids });
  } catch (e) {
    fail(e);
  } finally {
    app.pinging = false;
  }
}

export async function connect() {
  if (!app.servers.length) {
    app.popup = { kind: 'add' };
    return;
  }
  if (app.settings.mode === 'tun' && app.relaunchForTun) {
    app.popup = { kind: 'admin' };
    return;
  }
  const auto = !app.settings.selected || app.settings.selected === 'auto';
  if (auto && !app.servers.some((s) => s.ping > 0)) await pingAll();
  const server = target();
  if (!server) return;
  if (app.status.state === 'connecting' && app.status.server === server.id) return;
  try {
    // "auto" is the core's own choice among all servers (a urltest group); it reports which one it took
    await invoke('connect', { id: auto ? 'auto' : server.id });
  } catch {
    // the reason arrives with the status event and is shown from there
  }
}

export const disconnect = () => invoke('disconnect').catch(fail);

export function toggle() {
  return app.status.state === 'off' ? connect() : disconnect();
}

export function select(id) {
  if (app.settings.selected === id && app.status.state !== 'off') return;
  set('selected', id);
  if (app.status.state !== 'off') connect();
}

async function busy(key, fn) {
  if (app.busy[key]) return;
  app.busy[key] = true;
  try {
    return await fn();
  } finally {
    delete app.busy[key];
  }
}

export function importText(text) {
  return busy('import', async () => {
    try {
      const r = await invoke('import_text', { text });
      const parts = [];
      if (r.subs) parts.push(`подписок: ${r.subs}`);
      if (r.servers) parts.push(`серверов: ${r.servers}`);
      toast(`Добавлено — ${parts.join(', ')}`, 'ok');
      for (const e of r.errors ?? []) toast(e, 'error');
      if (r.skipped) toast(`Пропущено неподдерживаемых ссылок: ${r.skipped}`);
      pingAll();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  });
}

export function refreshSub(id) {
  return busy('sub:' + id, async () => {
    try {
      await invoke('refresh_subscription', { id });
      toast('Подписка обновлена', 'ok');
      pingAll(app.servers.filter((s) => s.sub === id).map((s) => s.id));
    } catch (e) {
      fail(e);
    }
  });
}

export const refreshAll = () => Promise.all(app.subs.map((s) => refreshSub(s.id)));

export const deleteSub = (id) => invoke('delete_subscription', { id }).catch(fail);
export const deleteServer = (id) => invoke('delete_server', { id }).catch(fail);

export async function locateHome() {
  try {
    await invoke('locate_home');
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

let appsLoading = null;
/** The programs installed here, for the exclusions: `[{id, name, icon?}]`. Asked for once. */
export function loadApps() {
  appsLoading ??= invoke('list_apps')
    .then((list) => (app.apps = list ?? []))
    .catch(() => (app.apps = []));
  return appsLoading;
}

// ---- geo data (docs/geo-api.md). The files are not in the state snapshot; the list is asked
// for when a screen needs it and replaced with what each command returns.

let geoLoading = null;
export function loadGeo() {
  geoLoading ??= invoke('geo_list')
    .then((list) => {
      app.geo = list ?? [];
      app.geoError = '';
    })
    .catch((e) => (app.geoError = errText(e)))
    .finally(() => (geoLoading = null));
  return geoLoading;
}

function putGeo(file) {
  if (!app.geo || !file) return;
  const i = app.geo.findIndex((f) => f.id === file.id);
  if (i >= 0) app.geo[i] = file;
  else app.geo.push(file);
}

const added = (f) =>
  f.kind === 'ruleset' ? `Набор правил ${f.name} добавлен` : `Файл ${f.name} добавлен, в нём ${many(f.categories, 'категория', 'категории', 'категорий')}`;

export function geoAddFile() {
  return busy('geo:file', async () => {
    let path;
    try {
      path = await pickFile([{ name: 'Гео-данные', extensions: ['dat', 'srs', 'json'] }]);
    } catch (e) {
      fail(e);
      return;
    }
    if (!path) return;
    try {
      const f = await invoke('geo_add_file', { path });
      putGeo(f);
      toast(added(f), 'ok');
    } catch (e) {
      fail(e);
    }
  });
}

/** Resolves to '' when the file was added, or to the reason it was not. */
export function geoAddUrl(url, autoUpdate) {
  return busy('geo:url', async () => {
    try {
      const f = await invoke('geo_add_url', { url, autoUpdate });
      putGeo(f);
      toast(added(f), 'ok');
      return '';
    } catch (e) {
      return errText(e);
    }
  });
}

export async function geoSetEnabled(id, enabled) {
  const f = app.geo?.find((x) => x.id === id);
  if (!f) return;
  f.enabled = enabled;
  try {
    putGeo(await invoke('geo_set_enabled', { id, enabled }));
  } catch (e) {
    f.enabled = !enabled;
    fail(e);
  }
}

export async function geoRemove(id) {
  try {
    await invoke('geo_remove', { id });
    app.geo = (app.geo ?? []).filter((f) => f.id !== id);
  } catch (e) {
    fail(e);
  }
}

/** One file, or every file that has somewhere to be downloaded from when `id` is left out. */
export function geoUpdate(id) {
  return busy('geo:' + (id ?? 'all'), async () => {
    try {
      const list = await invoke('geo_update', id ? { id } : {});
      app.geo = list;
      if (id) {
        const f = list.find((x) => x.id === id);
        if (f?.error) toast(`${f.name}: ${f.error}`, 'error');
        else if (f) toast(`${f.name} обновлён`, 'ok');
      } else {
        const bad = list.filter((f) => f.error && f.url);
        if (bad.length) toast(`Не обновились: ${bad.map((f) => f.name).join(', ')}`, 'error');
        else toast('Гео-данные обновлены', 'ok');
      }
    } catch (e) {
      fail(e);
    }
  });
}

/** The notice about dropped refs: open the settings right at the geo data. */
export function openGeo() {
  app.popup = null;
  app.panel = 'settings';
  app.geoFocus = true;
}

/** Switches on and downloads the files the dropped refs may come from. */
export function geoFixMissing() {
  return busy('geo:fix', async () => {
    const missing = app.geoMissing;
    if (!missing) return;
    if (!app.geo) await loadGeo();
    const files = fixable(missing.refs, app.geo ?? []);
    let ok = 0;
    for (const f of files) {
      try {
        if (!f.enabled) putGeo(await invoke('geo_set_enabled', { id: f.id, enabled: true }));
        if (f.url) {
          app.geo = await invoke('geo_update', { id: f.id });
          const now = app.geo.find((x) => x.id === f.id);
          if (now?.error) {
            toast(`${f.name}: ${now.error}`, 'error');
            continue;
          }
        }
        ok += 1;
      } catch (e) {
        fail(e);
      }
    }
    if (ok && app.geoMissing === missing) {
      missing.fixed = true;
      toast('Файлы скачаны. Правила заработают после переподключения', 'ok');
    }
  });
}

export async function copy(text, done = 'Скопировано') {
  try {
    await navigator.clipboard.writeText(text);
    toast(done, 'ok');
  } catch {
    toast('Не удалось скопировать', 'error');
  }
}

export async function init() {
  applyData(await invoke('get_state'), true);

  await listen('status', (s) => {
    const before = app.status.state;
    app.status = s;
    if (s.state === 'off') {
      app.traffic = { up: 0, down: 0, tup: 0, tdown: 0 };
      app.history = [];
    }
    if (s.error && before !== 'off') toast(s.error, 'error');
    // a new connection reports its own dropped geo refs, if any
    if (s.state === 'connecting' && before !== 'connecting') app.geoMissing = null;
  });
  await listen('geo-missing', (m) => {
    const refs = [...new Set(m?.refs ?? [])];
    app.geoMissing = refs.length ? { server: m.server ?? '', refs, fixed: false } : null;
    if (refs.length && app.geo) loadGeo();
  });
  await listen('data', (d) => {
    applyData(d);
    const fresh = app.servers.filter((s) => s.ping == null).map((s) => s.id);
    if (fresh.length) pingAll(fresh);
  });
  await listen('ping', ({ id, ms }) => {
    const s = serverById(id);
    if (s) s.ping = ms;
  });
  await listen('traffic', (t) => {
    app.traffic = t;
    app.history = [...app.history.slice(-(HISTORY - 1)), { up: t.up, down: t.down }];
  });
  let logQueued = false;
  await listen('log', (line) => {
    logLines.push(line);
    if (logLines.length > 1500) logLines.splice(0, logLines.length - 1500);
    if (!logQueued) {
      logQueued = true;
      requestAnimationFrame(() => {
        logQueued = false;
        app.logTick += 1;
      });
    }
  });
  await listen('tray-toggle', toggle);
  logLines.push(...(await invoke('get_logs')));

  setInterval(() => {
    if (app.status.state === 'on') app.now = Date.now();
  }, 1000);

  app.ready = true;

  if (!app.settings.home && app.settings.geo_lookup && app.status.state === 'off') invoke('locate_home').catch(() => {});
  if (app.servers.length) pingAll();
  if (app.status.state === 'off' && (app.settings.autoconnect || (app.elevated && app.settings.mode === 'tun'))) connect();
}
