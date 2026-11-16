# Geo data: the backend contract

Quadra keeps geo data files (v2ray `geosite` / `geoip` `.dat`, sing-box rule-sets) under the
app data dir, in `geo/`. Both cores use them: Xray reads the `.dat` files directly, sing-box
gets rule-sets converted from them on demand. This file is the contract between the Rust side
(`src-tauri/src/geo.rs`, commands in `lib.rs`) and the UI. Do not rename anything here without
changing both sides.

Calls go through `invoke(name, args)`. Tauri 2 turns snake_case Rust argument names into
camelCase on the JS side, so the argument names below are the ones to pass from JS.

## GeoFile

```ts
type GeoFile = {
  id: string;            // stable; built-ins have fixed ids ("ru-geosite", "ru-geoip", ...),
                         // user files get "u-" + 8 hex chars
  name: string;          // file name, also the name used in `ext:<name>:<tag>` refs
  kind: "geosite" | "geoip" | "ruleset";
  source: "builtin" | "file" | "url";
  url?: string;          // where it is downloaded from (built-ins and url sources)
  path: string;          // absolute path of the stored copy; "" while a built-in is not downloaded
  size: number;          // bytes; 0 while not downloaded
  updated_at: number;    // unix seconds of the last successful import or download; 0 = never
  categories: number;    // tags in the file; 1 for a rule-set; 0 while not downloaded
  auto_update: boolean;  // refreshed daily when settings.geo_auto_update is on
  enabled: boolean;      // disabled files are not used by either core
  error?: string;        // last download or parse error, in Russian, for display
  aliases?: string[];    // other names this file answers to in `ext:` refs (built-ins)
  description?: string;  // one line in Russian, built-ins only
};
```

## Commands

| command | args (JS) | returns |
|---|---|---|
| `geo_list` | none | `GeoFile[]` (built-ins first, then user files in the order added) |
| `geo_add_file` | `{ path: string }` | `GeoFile` |
| `geo_add_url` | `{ url: string, autoUpdate: boolean }` | `GeoFile` (downloads before returning) |
| `geo_remove` | `{ id: string }` | `null`; built-ins return an error, disable them instead |
| `geo_set_enabled` | `{ id: string, enabled: boolean }` | `GeoFile` |
| `geo_update` | `{ id?: string }` | `GeoFile[]` (all files when `id` is omitted) |
| `geo_categories` | `{ id: string, query?: string, limit?: number }` | `{ tag: string, count: number }[]` |

Notes:
- Errors are rejected promises with a Russian message string, like the other commands.
- `geo_add_file` copies the file into `geo/`. The kind is detected by parsing, not by the
  extension: a `.dat` with domain entries is `geosite`, with CIDR entries `geoip`; a sing-box
  binary rule-set (`.srs`) or a source rule-set (`.json`) is `ruleset`. Anything else is an error.
  A file whose name is already taken gets a numeric suffix (`name-2.dat`).
- `geo_add_url` accepts the same kinds from `http(s)://` URLs. Updates use ETag /
  If-Modified-Since; they go through the active connection when one is up and fall back to a
  direct request.
- `geo_categories`: `query` filters tags by substring, case-insensitive; `limit` defaults to 500.
  Sorted by tag. Tags are lower-case; `count` is the number of domains (geosite) or networks
  (geoip) in the tag. For a rule-set it returns one entry: `{ tag: <name without extension>,
  count: <domains and networks in it> }`. A file that is not downloaded yet is an error.
- `geo_update` never throws for a single failed file; it sets that file's `error` and returns
  the list.

## Event

`geo-missing`, payload `{ server: string, refs: string[] }`.
Sent on connect when the config (the provider's or the user's exclusions) referred to geo data
that could not be resolved. Those rules were dropped and the core started anyway. `server` is
the server's display name, `refs` the refs as written (`"geosite:foo"`, `"ext:bar.dat:baz"`).

The `data` event and `get_state` are unchanged; geo files are not part of the state snapshot,
call `geo_list` when the screen opens and after each command.

## Settings

`settings.geo_auto_update: boolean`, default `true`. Saved with `save_settings` like the rest.

## Refs accepted in exclusions and rules

`settings.bypass_domains` and domain/ip `settings.rules` accept, besides plain domains and IPs:
- `geosite:<tag>` (with `@attr` filters, e.g. `geosite:google@ads`), `geoip:<tag>`,
  `geoip:!<tag>`;
- `ext:<file>:<tag>` (Xray's syntax; `<file>` is a GeoFile `name` or one of its `aliases`);
- `ruleset:<name>` for a rule-set file (`<name>` = file name without extension).

A `geosite:` or `geoip:` tag is looked up in the default files first, then in every enabled
file of that kind, so `geosite:ru-blocked` works once the runetfreedom set is there.
An `ext:` ref to a known community file name that is not present yet is downloaded from its
built-in source before connecting.

## Built-in sets

Ids are fixed; the list may grow. Only the two defaults are there from the start (copied from
the files that ship with Xray). The others are downloaded when a config needs them: an `ext:`
ref names the file or one of its aliases, or a plain `geosite:` / `geoip:` tag the files at hand
lack is one the set is known to carry (`geosite:ru-blocked` brings `ru-geosite`). `geo_update`
with the id downloads one at any time.

| id | name (`ext:` file) | kind | source |
|---|---|---|---|
| `default-geosite` | `geosite.dat` | geosite | Loyalsoldier/v2ray-rules-dat (what the shipped Xray came with) |
| `default-geoip` | `geoip.dat` | geoip | Loyalsoldier/v2ray-rules-dat |
| `ru-geosite` | `geosite_RU.dat` | geosite | runetfreedom/russia-v2ray-rules-dat, about 90 MB |
| `ru-geoip` | `geoip_RU.dat` | geoip | runetfreedom/russia-v2ray-rules-dat |
| `roscomvpn-geosite` | `roscomvpn-geosite.dat` | geosite | hydraponique/roscomvpn-geosite |
| `roscomvpn-geoip` | `roscomvpn-geoip.dat` | geoip | hydraponique/roscomvpn-geoip |
| `zkeen` | `zkeen.dat` | geosite | jameszeroX/zkeen-domains |
| `zkeenip` | `zkeenip.dat` | geoip | jameszeroX/zkeen-ip |
| `zapret` | `zapret.dat` | detected | kutovoys/ru_gov_zapret |
| `v2fly-geosite` | `dlc.dat` | geosite | v2fly/domain-list-community |

A provider config may also carry Xray's own `"geodata": {"assets": [{"url", "file"}]}`
section; those files become `url` entries here (source `"url"`, auto-update on).

## Where things are

`<app data>/geo/`: the files under their `name`, `index.json` (the list), `tags/` (tag lists
by file hash), `rules/` (sing-box rule-sets made from `.dat` categories, by file hash and tag).
The folder is Xray's asset folder (`XRAY_LOCATION_ASSET`) for every connection.
