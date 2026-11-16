// Screenshots of every state of the geo data screens, from the real Svelte interface against the
// stand-in backend (src/lib/mock.js), into docs/img/geo/. No VPN connection is made.
//   node tools/capture-geo.mjs            (CHROME_PATH=... to pick the browser, PORT=... the dev port)
import { chromium } from 'playwright';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const out = resolve(root, 'docs/img/geo');
mkdirSync(out, { recursive: true });
const port = Number(process.env.PORT ?? 1431);
const base = `http://127.0.0.1:${port}/`;

const executablePath = [
  process.env.CHROME_PATH,
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
].filter(Boolean).find((p) => existsSync(p));

const server = spawn(process.execPath, [resolve(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: root,
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverLog = '';
server.stdout.on('data', (d) => (serverLog += d));
server.stderr.on('data', (d) => (serverLog += d));

function stopServer() {
  if (server.killed) return;
  if (process.platform === 'win32') spawnSync('taskkill', ['/pid', String(server.pid), '/t', '/f'], { stdio: 'ignore' });
  else server.kill('SIGTERM');
}

async function waitForServer() {
  for (let i = 0; i < 150; i++) {
    try {
      if ((await fetch(base)).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Vite did not start:\n${serverLog}`);
}

let browser;
let context;
const errors = [];

async function open(query = '', size = { width: 1440, height: 900 }) {
  const page = await context.newPage();
  await page.setViewportSize(size);
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  await page.goto(base + query);
  await page.waitForSelector('main', { state: 'visible' });
  await page.waitForTimeout(500);
  return page;
}

async function shot(page, name) {
  await page.screenshot({ path: resolve(out, name), animations: 'disabled' });
  console.log(`captured docs/img/geo/${name}`);
}

async function settingsAtGeo(page, wait = 700) {
  await page.locator('nav button[title="Настройки"]').click();
  await page.waitForTimeout(wait);
  await page.evaluate(() => {
    const h = [...document.querySelectorAll('h3')].find((e) => e.textContent.trim() === 'Гео-данные');
    h?.parentElement.scrollIntoView({ block: 'start' });
  });
  await page.waitForTimeout(250);
}

const fileRow = (page, name) => page.locator('.file .open', { hasText: name }).first();
const showAdd = (page) => page.evaluate(() => document.querySelector('.panel div.add').scrollIntoView({ block: 'center' }));

try {
  await waitForServer();
  browser = await chromium.launch({ headless: true, executablePath, args: ['--hide-scrollbars'] });
  context = await browser.newContext({ deviceScaleFactor: 1 });
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: base.slice(0, -1) });

  // ---- Settings, «Гео-данные»
  let page = await open();
  await settingsAtGeo(page);
  await shot(page, 'settings-geo.png');

  // the keyboard: focus walks the rows, Enter opens a file
  await page.getByRole('button', { name: 'Обновить geoip-ru.dat' }).focus();
  await page.keyboard.press('Tab');
  await page.waitForTimeout(150);
  await shot(page, 'settings-geo-focus.png');

  // update everything that has a source
  await page.getByRole('button', { name: 'Обновить все' }).click();
  await page.waitForTimeout(250);
  await shot(page, 'settings-geo-updating.png');
  await page.waitForTimeout(3500);
  await shot(page, 'settings-geo-updated.png');

  // switch a built-in on (it is not downloaded yet), then fetch it
  await page.getByRole('switch', { name: 'Использовать geosite-ru.dat' }).click();
  await page.getByRole('button', { name: 'Обновить geosite-ru.dat' }).click();
  await page.waitForTimeout(1300);
  await shot(page, 'settings-geo-builtin-fetched.png');

  // add from a URL: a wrong one, a failing one, then one that works
  const urlField = page.getByRole('textbox', { name: 'Ссылка на файл' });
  await showAdd(page);
  await urlField.fill('geosite.dat');
  await page.getByRole('button', { name: 'Добавить по ссылке' }).click();
  await page.waitForTimeout(200);
  await showAdd(page);
  await shot(page, 'settings-geo-url-invalid.png');
  await urlField.fill('https://example.net/404/geosite.dat');
  await page.getByRole('button', { name: 'Добавить по ссылке' }).click();
  await page.waitForTimeout(400);
  await showAdd(page);
  await shot(page, 'settings-geo-url-downloading.png');
  await page.waitForTimeout(1500);
  await showAdd(page);
  await shot(page, 'settings-geo-url-error.png');
  await urlField.fill('https://github.com/runetfreedom/russia-v2ray-rules-dat/releases/latest/download/geoip.dat');
  await page.getByRole('button', { name: 'Добавить по ссылке' }).click();
  await page.waitForTimeout(1800);
  await showAdd(page);
  await shot(page, 'settings-geo-url-added.png');

  // add a file through the dialog (the stand-in answers with a path at once)
  await page.getByRole('button', { name: 'Выбрать файл' }).click();
  await page.waitForTimeout(1000);
  await page.evaluate(() => document.querySelector('.panel .body').scrollBy(0, 400));
  await page.waitForTimeout(200);
  await shot(page, 'settings-geo-file-added.png');

  // remove asks first
  await page.getByRole('button', { name: 'Удалить antifilter.srs' }).click();
  await page.waitForTimeout(350);
  await shot(page, 'settings-geo-remove.png');
  await page.keyboard.press('Escape');

  // the global switch off: rows stop saying they update by themselves
  await page.getByRole('switch', { name: 'Обновлять гео-данные автоматически' }).click();
  await settingsAtGeo(page, 0);
  await shot(page, 'settings-geo-auto-off.png');
  await page.close();

  // ---- the category browser
  page = await open();
  await settingsAtGeo(page);
  await fileRow(page, 'geosite.dat').click();
  await page.waitForTimeout(600);
  await shot(page, 'categories.png');
  // a long list: only the visible rows exist, the keyboard pages through it
  for (let i = 0; i < 3; i++) await page.keyboard.press('PageDown');
  await page.keyboard.press('ArrowDown');
  await page.waitForTimeout(200);
  console.log('rows in the DOM for', await page.locator('.tag').count(), 'shown of', await page.locator('.sum').innerText());
  await shot(page, 'categories-keyboard.png');
  await page.getByRole('combobox').fill('you');
  await page.waitForTimeout(250);
  await shot(page, 'categories-search.png');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(300);
  await shot(page, 'categories-copied.png');
  console.log('clipboard:', await page.evaluate(() => navigator.clipboard.readText()));
  await page.getByRole('combobox').fill('zzzz');
  await page.waitForTimeout(250);
  await shot(page, 'categories-nothing-found.png');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  await fileRow(page, 'zkeen.dat').click();
  await page.waitForTimeout(500);
  await page.locator('.tag', { hasText: 'discord' }).click();
  await page.waitForTimeout(300);
  await shot(page, 'categories-ext.png');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  await fileRow(page, 'antifilter.srs').click();
  await page.waitForTimeout(500);
  await shot(page, 'categories-ruleset.png');
  await page.close();

  // ---- the routes screen: categories go into the sites list
  page = await open();
  await page.locator('nav button[title="Маршрутизация"]').click();
  await page.waitForTimeout(600);
  await page.getByRole('button', { name: 'Добавить категорию' }).click();
  await page.waitForTimeout(700);
  await page.getByRole('combobox').fill('telegram');
  await page.waitForTimeout(200);
  await page.keyboard.press('Enter');
  await page.getByRole('combobox').fill('youtube');
  await page.waitForTimeout(200);
  await page.keyboard.press('Enter');
  await page.getByRole('combobox').fill('');
  await page.getByRole('tab', { name: 'geoip.dat' }).click();
  await page.waitForTimeout(400);
  await page.getByRole('combobox').fill('ru');
  await page.waitForTimeout(200);
  await page.keyboard.press('Enter');
  await page.getByRole('combobox').fill('');
  await page.waitForTimeout(200);
  await shot(page, 'routes-picker.png');
  await page.getByRole('tab', { name: 'zkeen.dat' }).click();
  await page.waitForTimeout(400);
  await page.locator('.tag', { hasText: 'discord' }).click();
  await page.waitForTimeout(200);
  await shot(page, 'routes-picker-ext.png');
  await page.getByRole('button', { name: 'Готово' }).click();
  await page.waitForTimeout(400);
  await shot(page, 'routes-chips.png');
  await page.close();

  // ---- dropped refs after connecting
  page = await open('?geomissing&delay=600');
  await page.getByRole('button', { name: 'Подключить', exact: true }).click();
  await page.waitForTimeout(2000);
  await shot(page, 'missing-notice.png');
  await page.getByRole('button', { name: 'Показать', exact: true }).click();
  await page.waitForTimeout(900);
  await shot(page, 'missing-settings.png');
  await page.getByRole('button', { name: 'Скачать нужные' }).click();
  await page.waitForTimeout(400);
  await shot(page, 'missing-downloading.png');
  await page.waitForTimeout(2600);
  await shot(page, 'missing-fixed.png');
  await page.locator('nav button[title="Карта"]').click();
  await page.waitForTimeout(500);
  await shot(page, 'missing-notice-fixed.png');
  await page.close();

  page = await open('?geomissing=all&delay=600');
  await page.getByRole('button', { name: 'Подключить', exact: true }).click();
  await page.waitForTimeout(1600);
  await page.getByRole('button', { name: 'Показать', exact: true }).click();
  await page.waitForTimeout(900);
  await shot(page, 'missing-all-fixable.png');
  await page.close();

  // ---- empty, loading and error states
  for (const [query, name] of [['?geo=own-empty', 'state-no-own-files.png'], ['?geo=none', 'state-no-files.png'],
    ['?geo=slow', 'state-loading.png'], ['?geo=error', 'state-error.png']]) {
    page = await open(query);
    await settingsAtGeo(page);
    await shot(page, name);
    await page.close();
  }
  for (const [query, name] of [['?geo=tags-slow', 'categories-loading.png'], ['?geo=tags-error', 'categories-error.png']]) {
    page = await open(query);
    await settingsAtGeo(page);
    await fileRow(page, 'geosite.dat').click();
    await page.waitForTimeout(600);
    await shot(page, name);
    await page.close();
  }
  page = await open('?geo=none');
  await page.locator('nav button[title="Маршрутизация"]').click();
  await page.waitForTimeout(500);
  await page.getByRole('button', { name: 'Добавить категорию' }).click();
  await page.waitForTimeout(700);
  await shot(page, 'routes-picker-no-files.png');
  await page.close();

  // ---- light theme and a narrow window
  page = await open();
  await page.locator('nav button[title="Настройки"]').click();
  await page.waitForTimeout(500);
  await page.getByRole('radio', { name: 'Светлая' }).click();
  await settingsAtGeo(page, 0);
  await shot(page, 'settings-geo-light.png');
  await fileRow(page, 'geosite.dat').click();
  await page.waitForTimeout(600);
  await shot(page, 'categories-light.png');
  await page.close();

  page = await open('?platform=linux&tiling', { width: 520, height: 900 });
  await settingsAtGeo(page);
  await shot(page, 'settings-geo-narrow.png');
  await page.close();

  if (errors.length) {
    console.error('page errors:\n' + errors.join('\n'));
    process.exitCode = 1;
  }
} finally {
  await browser?.close();
  stopServer();
}
