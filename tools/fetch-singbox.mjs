// Builds sing-box (our fork with the XHTTP transport) for one platform and puts it, with the
// rule-sets the app's routing presets use, into src-tauri/resources/sing-box.
//   node tools/fetch-singbox.mjs [--target <rust target triple>] [--source <sing-box checkout>]
// The source is, in this order: --source, $SINGBOX_SRC, ../sing-box-xhttp next to this repo,
// or a clone of the repository and branch named in tools/singbox.source (into .singbox-src).
// Needs Go (the version the fork's go.mod asks for).
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const arg = (name) => {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : '';
};
const triple = arg('--target');
if (process.argv.includes('--android')) throw new Error('sing-box for Android is not built yet: the Android app still runs Xray');

const goos = triple ? (triple.includes('windows') ? 'windows' : triple.includes('apple') ? 'darwin' : 'linux')
  : { win32: 'windows', darwin: 'darwin', linux: 'linux' }[process.platform];
const goarch = triple ? (triple.startsWith('aarch64') ? 'arm64' : 'amd64') : { x64: 'amd64', arm64: 'arm64' }[process.arch];
if (!goos || !goarch) throw new Error(`no sing-box build for ${process.platform}-${process.arch}`);

let source = arg('--source') || process.env.SINGBOX_SRC || resolve(root, '..', 'sing-box-xhttp');
if (!existsSync(join(source, 'go.mod'))) {
  const [repo, branch] = readFileSync(join(root, 'tools', 'singbox.source'), 'utf8').trim().split(/\s+/);
  source = join(root, '.singbox-src');
  if (!existsSync(join(source, 'go.mod'))) {
    console.log(`cloning ${repo} (${branch})`);
    execFileSync('git', ['clone', '--depth', '1', '--branch', branch, repo, source], { stdio: 'inherit' });
  }
}
const describe = (() => {
  try {
    return execFileSync('git', ['-C', source, 'describe', '--tags', '--always', '--dirty'], { encoding: 'utf8' }).trim();
  } catch {
    return 'unknown';
  }
})();

const dest = join(root, 'src-tauri', 'resources', 'sing-box');
rmSync(dest, { recursive: true, force: true });
mkdirSync(join(dest, 'rules'), { recursive: true });
const binary = goos === 'windows' ? 'sing-box.exe' : 'sing-box';
const tags = 'with_gvisor,with_quic,with_utls,with_clash_api,badlinkname,tfogo_checklinkname0';
console.log(`sing-box ${describe} for ${goos}-${goarch} from ${source}`);
execFileSync('go', ['build', '-trimpath', '-tags', tags,
  '-ldflags', `-s -w -checklinkname=0 -X github.com/sagernet/sing-box/constant.Version=${describe.replace(/^v/, '')}`,
  '-o', join(dest, binary), './cmd/sing-box'],
  { cwd: source, stdio: 'inherit', env: { ...process.env, GOOS: goos, GOARCH: goarch, CGO_ENABLED: '0' } });
if (goos !== 'windows') chmodSync(join(dest, binary), 0o755);
if (existsSync(join(source, 'LICENSE'))) cpSync(join(source, 'LICENSE'), join(dest, 'LICENSE'));

// Rule-sets for the "Russia direct" preset (singbox.rs SHIPPED_RULE_SETS)
const sets = {
  'geosite-category-ru': 'https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs',
  'geoip-ru': 'https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-ru.srs',
};
for (const [name, url] of Object.entries(sets)) {
  const r = await fetch(url, { redirect: 'follow' });
  if (!r.ok) throw new Error(`${url}: ${r.status}`);
  const bytes = Buffer.from(await r.arrayBuffer());
  if (bytes.length < 1000) throw new Error(`${name}: suspiciously small (${bytes.length} bytes)`);
  writeFileSync(join(dest, 'rules', `${name}.srs`), bytes);
  console.log(name, bytes.length, 'bytes');
}
writeFileSync(join(dest, 'VERSION'), `${describe}\n`);
console.log('built into', dest);
