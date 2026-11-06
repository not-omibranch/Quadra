// Rebuild every README visual from the real Svelte interface. No VPN connection is made:
// the browser build uses src/lib/mock.js, including its simulated connect and traffic events.
import { chromium } from 'playwright';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(import.meta.dirname, '..');
const img = resolve(root, 'docs/img');
const frames = resolve(root, 'docs/.frames');
mkdirSync(img, { recursive: true });
mkdirSync(frames, { recursive: true });

const chromeCandidates = [
  process.env.CHROME_PATH,
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  '/usr/bin/google-chrome',
  '/usr/bin/chromium',
].filter(Boolean);
const executablePath = chromeCandidates.find((p) => existsSync(p));

const server = spawn(process.execPath, [resolve(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', '1420', '--strictPort'], {
  cwd: root,
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverLog = '';
server.stdout.on('data', (d) => { serverLog += d; });
server.stderr.on('data', (d) => { serverLog += d; });

function stopServer() {
  if (server.killed) return;
  if (process.platform === 'win32') spawnSync('taskkill', ['/pid', String(server.pid), '/t', '/f'], { stdio: 'ignore' });
  else server.kill('SIGTERM');
}

async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try {
      const response = await fetch('http://127.0.0.1:1420/');
      if (response.ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`Vite did not start:\n${serverLog}`);
}

async function loaded(page) {
  await page.waitForSelector('main', { state: 'visible' });
  await page.waitForTimeout(900);
}

async function shot(page, name) {
  await page.screenshot({ path: resolve(img, name), animations: 'disabled' });
  console.log(`captured docs/img/${name}`);
}

let browser;
try {
  await waitForServer();
  browser = await chromium.launch({ headless: true, executablePath, args: ['--hide-scrollbars'] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));

  await page.goto('http://127.0.0.1:1420/?delay=2400');
  await loaded(page);
  await shot(page, 'main.png');

  await page.getByRole('button', { name: 'Подключить', exact: true }).click();
  await page.mouse.move(720, 850);
  for (let i = 0; i < 32; i++) {
    await page.screenshot({ path: resolve(frames, `frame-${String(i).padStart(3, '0')}.png`) });
    await page.waitForTimeout(140);
  }
  await page.waitForSelector('button[aria-label="Отключить"]', { state: 'visible' });
  await page.waitForTimeout(1800);
  await shot(page, 'connected.png');

  await page.getByTitle('Маршрутизация', { exact: true }).click();
  await page.waitForSelector('text=Что идёт мимо сервера', { state: 'visible' });
  await shot(page, 'routes.png');

  await page.getByTitle('Настройки', { exact: true }).click();
  await page.waitForSelector('text=Акцентный цвет', { state: 'visible' });
  await shot(page, 'settings.png');

  await page.getByRole('radio', { name: 'Светлая', exact: true }).click();
  await page.waitForFunction(() => document.documentElement.dataset.theme === 'light');
  await page.getByTitle('Закрыть (Esc)', { exact: true }).click();
  await page.waitForTimeout(350);
  await shot(page, 'light.png');

  await page.getByTitle('Настройки', { exact: true }).click();
  await page.getByRole('radio', { name: 'Тёмная', exact: true }).click();
  await page.getByRole('button', { name: '#a78bfa', exact: true }).click();
  await page.getByTitle('Закрыть (Esc)', { exact: true }).click();
  await page.waitForTimeout(350);
  await shot(page, 'accent.png');

  const phone = await context.newPage();
  await phone.setViewportSize({ width: 430, height: 900 });
  await phone.goto('http://127.0.0.1:1420/?platform=android&delay=1600');
  await loaded(phone);
  await phone.getByRole('button', { name: 'Подключить', exact: true }).click();
  await phone.mouse.move(210, 100);
  await phone.waitForSelector('button[aria-label="Отключить"]', { state: 'visible' });
  await phone.waitForTimeout(1400);
  await shot(phone, 'android.png');
  await phone.close();

  const brand = await context.newPage();
  await brand.setViewportSize({ width: 1600, height: 500 });
  await brand.goto(pathToFileURL(resolve(root, 'docs/brand/banner.html')).href);
  await brand.evaluate(() => Promise.all([...document.images].map((image) => image.complete ? null : new Promise((resolve) => { image.onload = image.onerror = resolve; }))));
  await brand.screenshot({ path: resolve(img, 'banner.png') });
  console.log('captured docs/img/banner.png');

  await brand.setViewportSize({ width: 512, height: 512 });
  await brand.goto(pathToFileURL(resolve(root, 'docs/brand/logo.svg')).href);
  await brand.screenshot({ path: resolve(img, 'logo.png'), omitBackground: true });
  console.log('captured docs/img/logo.png');

  await brand.setViewportSize({ width: 1400, height: 460 });
  await brand.goto(pathToFileURL(resolve(root, 'docs/brand/flow.svg')).href);
  await brand.screenshot({ path: resolve(img, 'how-it-works.png') });
  console.log('captured docs/img/how-it-works.png');

  if (errors.length) throw new Error(`Browser errors:\n${errors.join('\n')}`);
} finally {
  if (browser) await browser.close();
  stopServer();
}
