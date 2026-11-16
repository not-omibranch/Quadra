// Stand-in backend for running the interface in a browser. Mirrors the commands and events of
// src-tauri/src/main.rs closely enough to exercise every screen.
const handlers = new Map();
const emit = (name, payload) => (handlers.get(name) ?? []).forEach((fn) => fn(payload));

const srv = (id, sub, name, cc, proto, transport, security, ping, note = '') => ({
  id, sub, name, cc, proto, transport, security, address: `${id}.example.net`, port: 443, note, full: proto === 'xray', ping,
});

const state = {
  settings: {
    theme: 'dark', accent: '#3ddc84', effects: 'full', mode: 'proxy', socks_port: 20808, http_port: 20809,
    allow_lan: false, routing: 'lan', rules: [{ kind: 'domain', value: 'example.org', action: 'direct' }],
    bypass_mode: 'exclude', bypass_apps: ['Steam.exe'], bypass_domains: ['example.org', 'bank.example'],
    dns: '1.1.1.1, 8.8.8.8', close_to_tray: true, autostart: false, autoconnect: false, send_hwid: true,
    user_agent: '', geo_lookup: true, home: { lat: 55.75, lon: 37.62, cc: 'RU', ip: '203.0.113.7', manual: false },
    sub_update_hours: 12, log_level: 'warning', core: 'auto', verify_first: false, selected: 'a2', collapsed: [],
    geo_auto_update: true,
  },
  subs: [
    { id: 's1', url: 'https://sub.example/one', title: 'ArrowVPN', upload: 0, download: 6.4e9, total: 21474836480,
      expire: Math.floor(Date.now() / 1000) + 59 * 86400, support_url: 'https://t.me/example', web_url: '',
      announce: 'Трафик 20 Гб в день\nОсталось дней: 59', update_hours: 1, updated_at: Math.floor(Date.now() / 1000) - 1300, error: '' },
    { id: 's2', url: 'https://sub.example/two', title: 'Total VPN', upload: 0, download: 0, total: 0,
      expire: Math.floor(Date.now() / 1000) + 2 * 86400, support_url: '', web_url: '',
      announce: 'Подписка действует еще 2 дней', update_hours: 1, updated_at: Math.floor(Date.now() / 1000) - 90000,
      error: 'у провайдера занят лимит устройств: удали старое устройство в личном кабинете' },
  ],
  servers: [
    srv('a1', 's1', 'Великобритания', 'GB', 'vless', 'tcp', 'reality', 71),
    srv('a2', 's1', 'Германия', 'DE', 'vless', 'tcp', 'reality', 48),
    srv('a3', 's1', 'Швейцария', 'CH', 'vless', 'tcp', 'reality', 55),
    srv('a4', 's1', 'Польша', 'PL', 'vless', 'tcp', 'reality', 39),
    srv('a5', 's1', 'Нидерланды', 'NL', 'vless', 'tcp', 'reality', 52),
    srv('a6', 's1', 'США (для ИИ)', 'US', 'vless', 'tcp', 'reality', 142),
    srv('a7', 's1', 'Индия', 'IN', 'vless', 'tcp', 'reality', 188),
    srv('a8', 's1', 'Швеция', 'SE', 'shadowsocks', 'tcp', 'none', -1),
    srv('b1', 's2', 'Авто-выбор', 'EU', 'xray', 'tcp', 'reality', 44, 'Лучший сервер'),
    srv('b2', 's2', 'Эстония', 'EE', 'vless', 'xhttp', 'tls', 36),
    srv('b3', 's2', 'Финляндия', 'FI', 'vless', 'ws', 'tls', 33),
    srv('b4', 's2', 'США | Gemini', 'US', 'vless', 'tcp', 'reality', 151),
    srv('b5', 's2', 'Япония', 'JP', 'vless', 'grpc', 'reality', 210),
    srv('b6', 's2', 'Бразилия', 'BR', 'trojan', 'tcp', 'tls', 240),
    srv('m1', null, 'Свой сервер', 'TR', 'vless', 'tcp', 'reality', null),
  ],
  status: { state: 'off', server: null, since: 0, ms: null, check: '', error: null, exit: null, tun: false },
  hwid: '6F3A1C22-9B4E-47D0-A1F8-5C2E7D903B16',
  admin: false,
  elevated: false,
  platform: new URLSearchParams(location.search).get('platform') ?? 'windows',
  tiling: new URLSearchParams(location.search).has('tiling'),
  relaunchForTun: (new URLSearchParams(location.search).get('platform') ?? 'windows') === 'windows',
  version: '0.4.0',
  userAgent: 'Quadra/0.4.0',
};

const snapshot = () => JSON.parse(JSON.stringify(state));
const logs = ['Xray 26.9.30 started', '[Warning] core: sample warning line'];
let traffic = null;
let pending = null;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const params = new URLSearchParams(location.search);

// ---- geo data (docs/geo-api.md). ?geo=none | own-empty | slow | error | tags-slow | tags-error
// picks a state to look at; ?geomissing (or =all) makes a connection report dropped refs;
// ?pick=cancel closes the file dialog without a choice.
const geoMode = params.get('geo') ?? '';
const ISO = ('ad ae af ag al am ao ar at au az ba bb bd be bf bg bh bi bj bn bo br bs bt bw by bz ca cd cf cg ch ci cl cm cn co cr cu cv cy cz ' +
  'de dj dk dm do dz ec ee eg er es et fi fj fm fr ga gb gd ge gh gm gn gq gr gt gw gy hk hn hr ht hu id ie il in iq ir is it jm jo jp ke kg kh ' +
  'ki km kn kp kr kw kz la lb lc li lk lr ls lt lu lv ly ma mc md me mg mh mk ml mm mn mo mr mt mu mv mw mx my mz na ne ng ni nl no np nr nz om ' +
  'pa pe pg ph pk pl ps pt pw py qa ro rs ru rw sa sb sc sd se sg si sk sl sm sn so sr ss st sv sy sz td tg th tj tl tm tn to tr tt tv tw tz ' +
  'ua ug us uy uz va vc ve vn vu ws ye za zm zw').split(' ');
const SITES = ('0x0 115 1password 2ch 2kgames 36kr 4chan 4pda 9gag abc abema acer adguard adobe adobe-activation agoda airbnb akamai alibaba ' +
  'alibabacloud aliexpress amazon amd anthropic apkpure apple apple-cn apple-dev applemusic archive arstechnica atlassian autodesk avast avito ' +
  'aws aws-cn azure baidu bbc behance bestbuy bilibili binance bing bitbucket bitwarden blizzard bloomberg booking box brave bybit bytedance canva ' +
  'category-ads category-ads-all category-ai-!cn category-anticensorship category-bank-ru category-communication category-cryptocurrency ' +
  'category-dev category-entertainment category-games category-gov-ru category-media category-media-ru category-ru ' +
  'category-social-media-!cn category-speedtest category-vpnservices chatgpt cisco claude cloudflare cloudflare-cn cn coinbase coursera ' +
  'crunchyroll deepl deezer dell deviantart digitalocean discord disney docker dropbox duckduckgo ea ebay elevenlabs epicgames etsy facebook ' +
  'fandom fastly figma firebase flickr gemini geolocation-!cn geolocation-cn github gitlab gmail godaddy google google-cn google-deepmind ' +
  'google-play google-scholar googlefcm gosuslugi grammarly habr hbo hetzner hp huawei hulu ibm icloud imdb imgur instagram intel jetbrains ' +
  'kaspersky kinopoisk kufar lenovo line linkedin mail-ru mailru-group medium mega meta microsoft microsoft-dev midjourney mozilla netflix ' +
  'nintendo notion nvidia okko onedrive openai opera oracle ozon patreon paypal perplexity pinterest pixiv playstation private protonmail ' +
  'python quora rambler reddit riot roblox ru rutracker rutube samsung sberbank signal skype slack sony soundcloud spotify stackexchange steam ' +
  'telegram tencent tesla tidal tiktok tinkoff tor twitch twitter ubisoft ubuntu udemy unity vk vimeo wargaming whatsapp wikimedia wikipedia ' +
  'wildberries x xbox xiaomi yahoo yandex youtube zoom').split(' ');
// more of the services a real geosite file has, so the list runs past a thousand rows like the real one
const SITES_MORE = ('abebooks accuweather adidas airtable alipay amazontrust ameba anydesk aol apache apkmirror appsflyer asus audible ' +
  'backblaze bahamut bandcamp battlenet blogger bluesky booth buffer buzzfeed canonical cbs chess chrome cnn codepen comodo crashlytics ' +
  'cursor cyberghost dailymotion dazn debian deepseek dji docusign douban doubleclick dribbble duolingo dw economist edge eff electron ' +
  'elsevier envato epson espn eurosport evernote expressvpn f-droid faceit fedora flipboard forbes fortinet freebsd garena genshin getty ' +
  'gitbook gitee glassdoor gog goodreads grafana groq guardian hackernews hashicorp heroku hoyoverse hsbc huggingface humble icq ifixit ' +
  'indeed inoreader intercom ixbt jsdelivr kaggle kakao kick kinguin kodi kotlin lastpass lazada letterboxd libreoffice linode lichess ' +
  'logitech mastodon mapbox matrix meduza megafon messenger miro mistral mojang mongodb msn myanimelist namecheap naver nexusmods nhk ' +
  'niconico nike nordvpn npmjs nytimes obsidian okta opensea openwrt origin outlook overleaf parsec pcloud pexels pikabu plex poe postman ' +
  'primevideo proton rakuten razer readthedocs replit reuters rockstar rust-lang salesforce scaleway scribd sentry shopify sketchfab ' +
  'snapchat sourceforge splice stripe substack supercell surfshark synology tailscale teamviewer threads tradingview trello tumblr ' +
  'tutanota uber unsplash upwork valve vercel viber vivaldi vultr walmart wechat weibo wetransfer wire wolfram wordpress yelp zendesk ' +
  'zhihu zoho').split(' ');
const ZKEEN = ('anthropic chatgpt claude cloudflare discord gemini github google-play instagram jetbrains linkedin meta netflix notion ' +
  'openai patreon rutracker soundcloud spotify telegram tiktok twitch twitter whatsapp x youtube zoom').split(' ');
const RU_SITES = ('category-ads-all discord google-deepmind meta no-russia-hosts ru-available-only-inside ru-blocked ru-blocked-all ' +
  'speedtest telegram twitter win-extra win-spy win-update youtube').split(' ');
const RU_IPS = 'cloudflare cloudfront ddos-guard discord facebook google private re-filter ru ru-blocked ru-blocked-community telegram yandex'.split(' ');

function hashed(tag, lo, hi) {
  let h = 2166136261;
  for (const c of tag) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  h = Math.imul(h ^ (h >>> 15), 2246822507) >>> 0;
  return lo + (h % (hi - lo));
}
const big = { cn: 64210, 'category-ads-all': 9871, 'geolocation-!cn': 23544, 'geolocation-cn': 4120, ru: 2304, 'ru-blocked': 41870, 'ru-blocked-all': 93112, us: 21987 };
const catalog = (tags, lo, hi) => tags.map((tag) => ({ tag, count: big[tag] ?? hashed(tag, lo, hi) })).sort((a, b) => (a.tag < b.tag ? -1 : 1));
const geoDir = 'C:\\Users\\me\\AppData\\Roaming\\dev.quadra.client\\geo\\';
const now = () => Math.floor(Date.now() / 1000);
const geoTags = {
  geosite: catalog([...new Set([...SITES, ...SITES_MORE, ...ISO.map((c) => `tld-${c}`)])], 3, 900),
  geoip: catalog([...ISO, 'private', 'cloudflare', 'cloudfront', 'facebook', 'fastly', 'google', 'netflix', 'telegram', 'twitter'], 2, 4000),
  'ru-geosite': catalog(RU_SITES, 20, 3000),
  'ru-geoip': catalog(RU_IPS, 4, 9000),
  'u-3fa91c02': catalog(ZKEEN, 6, 260),
  'u-81d0e5aa': [{ tag: 'antifilter', count: 31244 }],
  'u-c45b7e19': [{ tag: 'discord-voice', count: 412 }],
};
const geoFile = (f) => ({ path: '', size: 0, updated_at: 0, categories: 0, auto_update: false, enabled: true, ...f });
const geoSizes = {};
let geoFiles = [
  geoFile({ id: 'geosite', name: 'geosite.dat', kind: 'geosite', source: 'builtin', url: 'https://github.com/v2fly/domain-list-community/releases/latest/download/dlc.dat',
    path: geoDir + 'geosite.dat', size: 2861205, updated_at: now() - 3 * 3600, categories: geoTags.geosite.length, auto_update: true,
    aliases: ['dlc.dat'], description: 'Сайты по категориям: сервисы, реклама, страны' }),
  geoFile({ id: 'geoip', name: 'geoip.dat', kind: 'geoip', source: 'builtin', url: 'https://github.com/v2fly/geoip/releases/latest/download/geoip.dat',
    path: geoDir + 'geoip.dat', size: 19840512, updated_at: now() - 3 * 3600, categories: geoTags.geoip.length, auto_update: true,
    description: 'Адреса по странам и крупным сервисам' }),
  geoFile({ id: 'ru-geosite', name: 'geosite-ru.dat', kind: 'geosite', source: 'builtin', enabled: false, auto_update: true,
    url: 'https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geosite.dat', aliases: ['runetfreedom-geosite.dat'],
    description: 'Российские списки: заблокированное, доступное только из России' }),
  geoFile({ id: 'ru-geoip', name: 'geoip-ru.dat', kind: 'geoip', source: 'builtin', enabled: false, auto_update: true,
    url: 'https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geoip.dat', aliases: ['runetfreedom-geoip.dat'],
    description: 'Российские списки адресов: заблокированные сети' }),
  geoFile({ id: 'u-3fa91c02', name: 'zkeen.dat', kind: 'geosite', source: 'url', url: 'https://github.com/jameszeroX/zkeen-domains/releases/latest/download/zkeen.dat',
    path: geoDir + 'zkeen.dat', size: 412331, updated_at: now() - 26 * 3600, categories: geoTags['u-3fa91c02'].length, auto_update: true }),
  geoFile({ id: 'u-81d0e5aa', name: 'antifilter.srs', kind: 'ruleset', source: 'file', path: geoDir + 'antifilter.srs', size: 1204118,
    updated_at: now() - 5 * 86400, categories: 1 }),
  geoFile({ id: 'u-c45b7e19', name: 'discord-voice.json', kind: 'ruleset', source: 'url', url: 'https://lists.example.org/discord-voice.json',
    path: geoDir + 'discord-voice.json', size: 18230, updated_at: now() - 6 * 86400, categories: 1, auto_update: true,
    error: 'последнее обновление не прошло: сервер ответил 404, работает прежняя копия' }),
];
for (const f of geoFiles) geoSizes[f.id] = f.size || 1_600_000 + hashed(f.id, 0, 900_000);
if (geoMode === 'none') geoFiles = [];
if (geoMode === 'own-empty') geoFiles = geoFiles.filter((f) => f.source === 'builtin');

const geoCopy = () => JSON.parse(JSON.stringify(geoFiles));
const geoById = (id) => {
  const f = geoFiles.find((x) => x.id === id);
  if (!f) throw 'такого файла уже нет';
  return f;
};

function geoNew(name, source, url) {
  const ext = name.split('.').pop().toLowerCase();
  if (!['dat', 'srs', 'json'].includes(ext)) throw 'это не гео-данные: подходят файлы .dat, .srs и .json';
  if (/broken|bad/.test(name)) throw 'файл не читается: это не geosite, не geoip и не набор правил sing-box';
  let unique = name;
  for (let i = 2; geoFiles.some((f) => f.name === unique); i++) unique = name.replace(/(\.[^.]+)$/, `-${i}$1`);
  const kind = ext === 'dat' ? (/ip/i.test(name) ? 'geoip' : 'geosite') : 'ruleset';
  const id = 'u-' + Math.random().toString(16).slice(2, 10);
  geoTags[id] = kind === 'ruleset' ? [{ tag: unique.replace(/\.[^.]+$/, ''), count: hashed(unique, 50, 9000) }]
    : kind === 'geoip' ? catalog(['ru', 'ua', 'by', 'kz', 'private', 'telegram'], 10, 4000)
    : catalog(['discord', 'youtube', 'telegram', 'twitter', 'meta', 'linkedin', 'rutracker', 'nnmclub', 'chatgpt', 'zapret-hosts'], 4, 400);
  const size = kind === 'ruleset' ? hashed(unique, 9000, 900000) : hashed(unique, 80000, 3000000);
  const f = geoFile({ id, name: unique, kind, source, url, path: geoDir + unique, size, updated_at: now(), categories: geoTags[id].length, auto_update: false });
  geoFiles.push(f);
  return f;
}

export async function pickFile() {
  await wait(300);
  return params.get('pick') === 'cancel' ? null : 'C:\\Users\\me\\Downloads\\geosite-zapret.dat';
}

function setStatus(patch) {
  Object.assign(state.status, patch);
  emit('status', { ...state.status });
}

function stopTraffic() {
  clearInterval(traffic);
  traffic = null;
}

const commands = {
  // ?boot=3000 holds the startup back, to look at the intro
  get_state: async () => {
    await wait(Number(params.get('boot') ?? 0));
    return snapshot();
  },
  ready: () => {},
  save_settings: ({ settings }) => { state.settings = settings; },
  async connect({ id }) {
    // the real core picks among all servers; the stand-in takes the lowest ping
    if (id === 'auto') id = [...state.servers].filter((s) => s.ping > 0).sort((a, b) => a.ping - b.ping)[0]?.id ?? state.servers[0]?.id;
    stopTraffic();
    const token = (pending = {});
    setStatus({ state: 'connecting', server: id, error: null, exit: null, ms: null });
    await wait(Number(params.get('delay') ?? 1500));
    if (pending !== token) return;
    const server = state.servers.find((s) => s.id === id);
    const dead = !server || server.ping === -1 || params.has('fail');
    const strict = state.settings.verify_first;
    if (!server || (dead && strict)) {
      setStatus({ state: 'off', error: 'сервер не отвечает: за 7600 мс через него не прошёл ни один запрос' });
      throw 'сервер не отвечает: за 7600 мс через него не прошёл ни один запрос';
    }
    // without verify_first the stand-in connects at once and reports the check a moment later
    setStatus({ state: 'on', since: Math.floor(Date.now() / 1000), ms: strict ? server.ping ?? 60 : null, check: strict ? 'ok' : 'checking' });
    if (!strict) setTimeout(() => pending === token && setStatus(dead ? { check: 'fail', ms: null } : { check: 'ok', ms: server.ping ?? 60 }), 1200);
    let tup = 0, tdown = 0, t = 0;
    traffic = setInterval(() => {
      t += 1;
      const down = Math.round((0.5 + 0.5 * Math.sin(t / 3)) * 4.2e6 * (0.6 + Math.random() * 0.8));
      const up = Math.round(down * 0.06);
      tup += up; tdown += down;
      emit('traffic', { up, down, tup, tdown });
    }, 1000);
    setTimeout(() => pending === token && setStatus({ exit: { ip: '198.51.100.24', cc: server.cc === 'EU' ? 'NL' : server.cc, lat: 52.37, lon: 4.9 } }), 600);
    // the provider's config asks for geo data this device lacks; the core starts without those rules
    if (params.has('geomissing')) {
      const refs = ['geosite:ru-blocked', 'geoip:ru-blocked'];
      if (params.get('geomissing') !== 'all') refs.push('ext:zapret.dat:discord');
      setTimeout(() => pending === token && emit('geo-missing', { server: server.name, refs }), 300);
    }
  },
  async disconnect() {
    pending = null;
    stopTraffic();
    setStatus({ state: 'off', since: 0, ms: null, check: '', error: null, exit: null });
  },
  async ping({ ids }) {
    for (const s of state.servers) {
      if (ids.length && !ids.includes(s.id)) continue;
      await wait(40);
      s.ping = s.id === 'a8' ? -1 : Math.round(25 + Math.random() * 220);
      emit('ping', { id: s.id, ms: s.ping });
    }
  },
  async add_subscription({ url }) {
    await wait(700);
    if (!/^https?:\/\//.test(url)) throw 'это не ссылка на подписку';
    const id = 's' + (state.subs.length + 1);
    state.subs.push({ id, url, title: new URL(url).host, upload: 0, download: 0, total: 0, expire: 0, support_url: '', web_url: '', announce: '', update_hours: 0, updated_at: Math.floor(Date.now() / 1000), error: '' });
    state.servers.push(srv(id + 'x', id, 'Франция', 'FR', 'vless', 'tcp', 'reality', null), srv(id + 'y', id, 'Канада', 'CA', 'vless', 'tcp', 'reality', null));
    emit('data', snapshot());
    return { servers: 2, skipped: 0 };
  },
  async refresh_subscription({ id }) {
    await wait(700);
    const sub = state.subs.find((s) => s.id === id);
    sub.updated_at = Math.floor(Date.now() / 1000);
    sub.error = '';
    emit('data', snapshot());
    return { servers: state.servers.filter((s) => s.sub === id).length, skipped: 0 };
  },
  async delete_subscription({ id }) {
    state.subs = state.subs.filter((s) => s.id !== id);
    state.servers = state.servers.filter((s) => s.sub !== id);
    emit('data', snapshot());
  },
  async delete_server({ id }) {
    state.servers = state.servers.filter((s) => s.id !== id);
    emit('data', snapshot());
  },
  async import_text({ text }) {
    await wait(400);
    if (/^https?:\/\//.test(text.trim())) return (await commands.add_subscription({ url: text.trim() })) && { servers: 0, subs: 1, skipped: 0, errors: [] };
    if (!text.includes('://')) throw 'здесь нет ни ссылок на серверы, ни подписок';
    state.servers.push(srv('m' + Date.now(), null, 'Импортированный', 'IT', 'vless', 'ws', 'tls', null));
    emit('data', snapshot());
    return { servers: 1, subs: 0, skipped: 0, errors: [] };
  },
  async locate_home() {
    await wait(500);
    state.settings.home = { lat: 55.75, lon: 37.62, cc: 'RU', ip: '203.0.113.7', manual: false };
    emit('data', snapshot());
    return state.settings.home;
  },
  share: ({ id }) => {
    const size = 29;
    let cells = '';
    for (let i = 0; i < size * size; i++) {
      const [x, y] = [i % size, Math.floor(i / size)];
      const finder = (x < 7 && y < 7) || (x > size - 8 && y < 7) || (x < 7 && y > size - 8);
      const fx = x < 7 ? x : x - (size - 7), fy = y < 7 ? y : y - (size - 7);
      cells += finder ? ((fx === 0 || fx === 6 || fy === 0 || fy === 6 || (fx > 1 && fx < 5 && fy > 1 && fy < 5)) ? '1' : '0') : ((x * 7 + y * 13 + x * y) % 3 === 0 ? '1' : '0');
    }
    return { text: `vless://00000000-0000-0000-0000-000000000000@${id}.example.net:443?type=tcp&security=reality#sample`, link: true, qr: { size, cells } };
  },
  open_url: () => true,
  async list_apps() {
    await wait(350);
    if (state.platform === 'android') {
      return ['Chrome|com.android.chrome', 'Telegram|org.telegram.messenger', 'YouTube|com.google.android.youtube', 'Банк|ru.bank.example',
        'Госуслуги|ru.gosuslugi.example', 'Карты|com.maps.example', 'Камера|com.android.camera', 'Музыка|com.music.example',
        'Почта|com.mail.example', 'Такси|com.taxi.example', 'Файлы|com.android.files', 'Часы|com.android.deskclock']
        .map((l) => ({ name: l.split('|')[0], id: l.split('|')[1] }));
    }
    if (state.platform !== 'windows') return [];
    return ['Discord|Discord.exe', 'Firefox|firefox.exe', 'Google Chrome|chrome.exe', 'Steam|Steam.exe', 'Telegram Desktop|Telegram.exe',
      'Visual Studio Code|Code.exe', 'qBittorrent|qbittorrent.exe', 'Яндекс Музыка|YandexMusic.exe']
      .map((l) => ({ name: l.split('|')[0], id: l.split('|')[1] }));
  },
  clipboard: () => 'vless://00000000-0000-0000-0000-000000000000@example.net:443?type=ws&security=tls#sample',
  get_logs: () => logs.slice(),
  clear_logs: () => { logs.length = 0; },
  relaunch_admin: async () => { throw 'запуск с правами администратора отменён'; },
  quit: () => {},

  async geo_list() {
    await wait(geoMode === 'slow' ? 1e9 : 250);
    if (geoMode === 'error') throw 'не удалось прочитать папку geo: отказано в доступе';
    return geoCopy();
  },
  async geo_add_file({ path }) {
    await wait(450);
    return JSON.parse(JSON.stringify(geoNew(path.split(/[\\/]/).pop(), 'file')));
  },
  async geo_add_url({ url, autoUpdate }) {
    await wait(1400);
    if (!/^https?:\/\//.test(url)) throw 'подходят только ссылки http:// и https://';
    if (/404/.test(url)) throw 'не скачался: сервер ответил 404';
    const name = decodeURIComponent(new URL(url).pathname.split('/').pop() || 'geo.dat');
    const f = geoNew(name, 'url', url);
    f.auto_update = !!autoUpdate;
    return JSON.parse(JSON.stringify(f));
  },
  async geo_remove({ id }) {
    if (geoById(id).source === 'builtin') throw 'встроенный файл нельзя удалить, его можно выключить';
    geoFiles = geoFiles.filter((f) => f.id !== id);
    return null;
  },
  async geo_set_enabled({ id, enabled }) {
    await wait(120);
    const f = geoById(id);
    f.enabled = enabled;
    return { ...f };
  },
  async geo_update({ id } = {}) {
    const targets = id ? [geoById(id)] : geoFiles.filter((f) => f.url);
    for (const f of targets) {
      await wait(id ? 900 : 450);
      if (!f.url) continue;
      if (f.url.includes('example.org')) {
        f.error = 'последнее обновление не прошло: сервер ответил 404, работает прежняя копия';
        continue;
      }
      f.path ||= geoDir + f.name;
      f.size = geoSizes[f.id] ?? f.size;
      f.categories = geoTags[f.id]?.length ?? f.categories;
      f.updated_at = now();
      delete f.error;
    }
    return geoCopy();
  },
  async geo_categories({ id, query = '', limit = 500 }) {
    await wait(geoMode === 'tags-slow' ? 1e9 : 160);
    if (geoMode === 'tags-error') throw 'файл повреждён: не удалось разобрать список категорий';
    const f = geoById(id);
    if (!f.path) throw 'файл ещё не скачан';
    const q = query.toLowerCase();
    return (geoTags[id] ?? []).filter((t) => !q || t.tag.includes(q)).slice(0, limit);
  },
};

export async function invoke(cmd, args = {}) {
  if (!commands[cmd]) throw `unknown command ${cmd}`;
  return commands[cmd](args);
}

export function listen(name, fn) {
  if (!handlers.has(name)) handlers.set(name, []);
  handlers.get(name).push(fn);
  return Promise.resolve(() => handlers.set(name, handlers.get(name).filter((f) => f !== fn)));
}

export const appWindow = {
  minimize: async () => {},
  toggleMaximize: async () => {},
  close: async () => {},
  isMaximized: async () => false,
  onResized: async () => () => {},
};
