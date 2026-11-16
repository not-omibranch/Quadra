// Builds sing-box (our fork with the XHTTP transport) for one platform and puts it, with the
// rule-sets the app's routing presets use, into src-tauri/resources/sing-box.
//   node tools/fetch-singbox.mjs [--target <rust target triple>] [--source <sing-box checkout>]
//   node tools/fetch-singbox.mjs --android [--abi arm64-v8a,armeabi-v7a,x86_64]
// The source is, in this order: --source, $SINGBOX_SRC, ../sing-box-xhttp next to this repo,
// or a clone of the repository and branch named in tools/singbox.source (into .singbox-src).
// Needs Go (the version the fork's go.mod asks for); --android also needs the Android NDK
// ($ANDROID_NDK_HOME, $NDK_HOME or the newest one under $ANDROID_HOME/ndk).
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const arg = (name) => {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : '';
};
const triple = arg('--target');
const android = process.argv.includes('--android');

const goos = triple ? (triple.includes('windows') ? 'windows' : triple.includes('apple') ? 'darwin' : 'linux')
  : { win32: 'windows', darwin: 'darwin', linux: 'linux' }[process.platform];
const goarch = triple ? (triple.startsWith('aarch64') ? 'arm64' : 'amd64') : { x64: 'amd64', arm64: 'arm64' }[process.arch];
if (!android && (!goos || !goarch)) throw new Error(`no sing-box build for ${process.platform}-${process.arch}`);

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

const tags = 'with_gvisor,with_quic,with_utls,with_clash_api,badlinkname,tfogo_checklinkname0';
const ldflags = `-s -w -checklinkname=0 -X github.com/sagernet/sing-box/constant.Version=${describe.replace(/^v/, '')}`;

/** The rule-sets of the "Russia direct" preset (singbox.rs SHIPPED_RULE_SETS), into `dir`. */
async function fetchRuleSets(dir) {
  mkdirSync(dir, { recursive: true });
  const sets = {
    'geosite-category-ru': 'https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ru.srs',
    'geoip-ru': 'https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-ru.srs',
  };
  for (const [name, url] of Object.entries(sets)) {
    const r = await fetch(url, { redirect: 'follow' });
    if (!r.ok) throw new Error(`${url}: ${r.status}`);
    const bytes = Buffer.from(await r.arrayBuffer());
    if (bytes.length < 1000) throw new Error(`${name}: suspiciously small (${bytes.length} bytes)`);
    writeFileSync(join(dir, `${name}.srs`), bytes);
    console.log(name, bytes.length, 'bytes');
  }
}

if (android) {
  // Android runs a program only from the package's native library folder, so the core goes in
  // as "libsingbox.so" for each architecture, next to Xray's "libxray.so"; the rule-sets travel
  // as assets and the VPN plugin unpacks them into the app's folder.
  // Built with the NDK's clang (cgo): Go links android/amd64 and android/arm only that way, and
  // arm64 is built the same way so that the three are alike.
  // No gvisor: on Android the system VPN's device is read by tun2proxy inside the app, and
  // sing-box never opens a TUN of its own there (see src-tauri/src/android.rs).
  const ndk = process.env.ANDROID_NDK_HOME || process.env.NDK_HOME || (() => {
    const base = join(process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT || '', 'ndk');
    const found = existsSync(base) ? readdirSync(base).sort((a, b) => a.localeCompare(b, undefined, { numeric: true })) : [];
    return found.length ? join(base, found[found.length - 1]) : '';
  })();
  if (!ndk || !existsSync(ndk)) throw new Error('no Android NDK found: set ANDROID_NDK_HOME');
  const host = { linux: 'linux-x86_64', darwin: 'darwin-x86_64', win32: 'windows-x86_64' }[process.platform];
  const bin = join(ndk, 'toolchains', 'llvm', 'prebuilt', host, 'bin');
  const api = 24; // the VPN plugin's minSdk (the app itself asks for 26)
  const all = {
    'arm64-v8a': { GOARCH: 'arm64', clang: `aarch64-linux-android${api}-clang` },
    'armeabi-v7a': { GOARCH: 'arm', GOARM: '7', clang: `armv7a-linux-androideabi${api}-clang` },
    x86_64: { GOARCH: 'amd64', clang: `x86_64-linux-android${api}-clang` },
  };
  const wanted = arg('--abi') ? arg('--abi').split(',') : Object.keys(all);
  const main = join(root, 'src-tauri', 'plugins', 'vpn', 'android', 'src', 'main');
  const androidTags = tags.replace('with_gvisor,', '');
  for (const abi of wanted) {
    const { clang, ...goenv } = all[abi] ?? {};
    if (!clang) throw new Error(`unknown ABI ${abi}`);
    const cc = join(bin, clang + (process.platform === 'win32' ? '.cmd' : ''));
    if (!existsSync(cc)) throw new Error(`${cc} is missing`);
    const out = join(main, 'jniLibs', abi, 'libsingbox.so');
    mkdirSync(dirname(out), { recursive: true });
    console.log(`sing-box ${describe} for android-${abi} from ${source}`);
    execFileSync('go', ['build', '-trimpath', '-buildvcs=false', '-tags', androidTags, '-ldflags', ldflags, '-o', out, './cmd/sing-box'],
      { cwd: source, stdio: 'inherit', env: { ...process.env, GOOS: 'android', CGO_ENABLED: '1', CC: cc, ...goenv } });
    console.log(`  ${out}: ${statSync(out).size} bytes`);
  }
  const assets = join(main, 'assets', 'sing-box');
  rmSync(assets, { recursive: true, force: true });
  await fetchRuleSets(join(assets, 'rules'));
  writeFileSync(join(assets, 'VERSION'), `${describe}\n`);
  if (existsSync(join(source, 'LICENSE'))) cpSync(join(source, 'LICENSE'), join(assets, 'LICENSE'));
  console.log('built into', main);
  process.exit(0);
}

const dest = join(root, 'src-tauri', 'resources', 'sing-box');
rmSync(dest, { recursive: true, force: true });
mkdirSync(join(dest, 'rules'), { recursive: true });
const binary = goos === 'windows' ? 'sing-box.exe' : 'sing-box';
console.log(`sing-box ${describe} for ${goos}-${goarch} from ${source}`);
execFileSync('go', ['build', '-trimpath', '-tags', tags, '-ldflags', ldflags, '-o', join(dest, binary), './cmd/sing-box'],
  { cwd: source, stdio: 'inherit', env: { ...process.env, GOOS: goos, GOARCH: goarch, CGO_ENABLED: '0' } });
if (goos !== 'windows') chmodSync(join(dest, binary), 0o755);
if (existsSync(join(source, 'LICENSE'))) cpSync(join(source, 'LICENSE'), join(dest, 'LICENSE'));
await fetchRuleSets(join(dest, 'rules'));
writeFileSync(join(dest, 'VERSION'), `${describe}\n`);
console.log('built into', dest);
