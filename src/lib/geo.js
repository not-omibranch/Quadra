// Geo data files and the refs rules use to point into them. The backend side and the exact
// formats are in docs/geo-api.md; everything here only reads GeoFile objects.
import { plural } from './format.js';

/** 1412 -> "1 412" */
export const num = (n) => (Number(n) || 0).toLocaleString('ru-RU');
export const many = (n, one, few, lots) => `${num(n)} ${plural(n, one, few, lots)}`;

export const KIND = { geosite: 'geosite', geoip: 'geoip', ruleset: 'набор правил' };
export const SOURCE = { builtin: 'встроенный', file: 'из файла', url: 'по ссылке' };

/** "antifilter.srs" -> "antifilter" */
export const stem = (name) => name.replace(/\.[^.]+$/, '');

export function host(url) {
  try {
    return new URL(url).host;
  } catch {
    return url ?? '';
  }
}

/** A file is usable when it is on disk, switched on and read without errors. */
export const ready = (f) => !!(f.enabled && f.path && !f.error);

/**
 * How a rule refers to one category of a file: the built-in sets answer to plain
 * `geosite:tag` / `geoip:tag`, a file of your own to Xray's `ext:<file>:<tag>`, a rule-set
 * to `ruleset:<name>` as a whole.
 */
export function refPrefix(file) {
  if (file.kind === 'ruleset') return 'ruleset:';
  if (file.source === 'builtin') return `${file.kind}:`;
  return `ext:${file.name}:`;
}

export const geoRef = (file, tag) => refPrefix(file) + (file.kind === 'ruleset' ? stem(file.name) : tag);

/** What the numbers next to a category count. */
export function unit(file, n) {
  if (file.kind === 'geoip') return many(n, 'сеть', 'сети', 'сетей');
  if (file.kind === 'ruleset') return many(n, 'правило', 'правила', 'правил');
  return many(n, 'домен', 'домена', 'доменов');
}

export const isRef = (value) => /^(geosite|geoip|ext|ruleset):/i.test(value);

/** "ext:zkeen.dat:youtube" -> { kind: "ext", file: "zkeen.dat", tag: "youtube" } */
export function parseRef(ref) {
  const ext = /^ext:([^:]+):(.+)$/i.exec(ref);
  if (ext) return { kind: 'ext', file: ext[1].toLowerCase(), tag: ext[2] };
  const m = /^(geosite|geoip|ruleset):!?(.+)$/i.exec(ref);
  return m ? { kind: m[1].toLowerCase(), tag: m[2] } : null;
}

/** The files a ref could be read from. */
export function suppliers(ref, files) {
  const r = parseRef(ref);
  if (!r) return [];
  if (r.kind === 'ext') {
    return files.filter((f) => f.name.toLowerCase() === r.file || (f.aliases ?? []).some((a) => a.toLowerCase() === r.file));
  }
  if (r.kind === 'ruleset') return files.filter((f) => f.kind === 'ruleset' && stem(f.name).toLowerCase() === r.tag.toLowerCase());
  return files.filter((f) => f.kind === r.kind);
}

/**
 * For refs the core could not resolve: the files that may hold them but are switched off,
 * not downloaded yet or broken. Switching those on and downloading them is all the app can
 * do by itself; refs with no such file need a file added by hand.
 */
export function fixable(refs, files) {
  const out = new Map();
  // a broken file added from disk has nowhere to be downloaded from again
  for (const ref of refs) for (const f of suppliers(ref, files)) if (!ready(f) && (f.url || (f.path && !f.error))) out.set(f.id, f);
  return [...out.values()];
}
