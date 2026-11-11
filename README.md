# Steamware — Rust + Tauri

The desktop client, rebuilt. Same design language as the WPF app, none of its network code: every request
goes to the Worker in [`../backend`](../backend), which holds the provider keys. See
[`../SECURITY.md`](../SECURITY.md) for the model.

## Status

Every page of the Windows build exists here, against the same sources, through the proxy:

| Page | This build | Notes |
| --- | --- | --- |
| Home | ✅ stats, Steam status, recent, usage, popular | |
| Store | ✅ search, genres, details, covers | |
| Library | ✅ Steam scan, covers, open folder, manifest | |
| Fixes | ✅ catalog, tags, downloads | shows "waiting for the provider key" until DepotBox is configured |
| Builds | ✅ depot/pin toggles, saved builds, raw lua editor | depot sizes and "latest build" come from Steam's app info via the Worker |
| Mode | ✅ OST / BetterSteamTools / Custom, verified installs, CloudRedirect add-on | closes and reopens Steam around a swap |
| Plugin | ✅ install/update/remove, Steamware v1.0.1, injector + local API | loads in Steam's store pages; reads an install made by the WPF app too |
| Downloads | ✅ streamed, progress, cancel, lua install | |
| Settings | ✅ Steamware appearance, Steam appearance, downloads, Steam, sources, account | Steam themes come from steambrew.app |

The store-page plugin is fully served by this build, with no WPF app needed alongside it:

- **Steamware, not the upstream bundle.** Install rewrites the frontend's prose to `Steamware`, repoints the
  old community link, swaps the manifest identity (`steamware`, `Steamware`, `1.0.1`) and its embedded icon,
  and removes the dead Lua backend. The same pass runs on load, so an older install is corrected in place.
  The app presents the plugin as **v1.0.1**, its own product version; the upstream release tag is only used
  to notice a changed bundle.
- **The injector.** A background task finds Steam's store tabs through CEF's debugging port, evaluates the
  frontend there, and re-injects after navigation or when the page was loaded by a previous run. It also
  adds the Steamware button emphasis, the Fix button and the menu skin, which live in
  [`src-tauri/resources/inject`](src-tauri/resources/inject).
- **The local API.** `127.0.0.1:6767` answers the plugin's `callServerMethod` calls (add status, lua
  presence, remove, fixes, settings, restart, links). It is loopback-only and every request must carry the
  per-run bridge token the injector holds; a local web page cannot call it.

The WPF build remains in `../LuaTools-main` and still works; nothing was deleted.

## What the desktop asks the Worker for

| Endpoint | Used by |
| --- | --- |
| `/v1/steam/app/:id` | Builds — depots, manifest ids, DLC, public build |
| `/v1/registry/release?repo=` | Mode and Plugin — latest release tag + asset digests |
| `/v1/registry/asset?repo=&tag=&name=` | Mode and Plugin — the archive/DLL itself, streamed |
| `/v1/registry/bst` | Mode — BetterSteamTools' version + payload hash |

Every one requires a session, and the asset route only serves four allowlisted repositories. GitHub is
never contacted by the app, so a user's IP stays out of GitHub's logs and a blocked region still works.

## Steam themes

Settings → **Steam Appearance** browses the themes published on steambrew.app and applies one to Steam's own
interface. There is no second loader, no Millennium install and nothing written into Steam's folder — themes
are CSS for Chromium windows, and this build already has a relay into those windows for its own plugin
(`injector.rs`), so [`src-tauri/src/themes.rs`](src-tauri/src/themes.rs) unpacks the theme into the profile
and the injector paints it into every Steam window the theme's own `skin.json` names.

- The catalog (`https://steambrew.app/api/v2`) is read directly, not through the Worker, and cached on disk
  for six hours. It is public metadata with no credential in it, and a theme archive is megabytes — keeping it
  off the user's own Worker budget is the point.
- Every URL a request is built from is checked against an allowlist of hosts themes actually come from
  (`github.com`, `codeload.github.com`, `objects.githubusercontent.com`, `raw.githubusercontent.com`,
  `steambrew.app`, `i.imgur.com`) and must be `https`.
- Unpacking is written by hand: only relative paths inside the theme's own folder are kept, and there are caps
  on the archive (64 MB), the unpacked total (48 MB) and any single file (8 MB).
- Each window is asked whether it is already wearing the current theme before anything is evaluated, so a theme
  costs one round trip per window and nothing after that.
- A theme's CSS is inlined with its images rewritten to `data:` URLs, because Steam's pages have a
  content-security policy and a relative `url(images/bg.png)` would otherwise resolve against the page.

Honest limits: themes style Steam's **Chromium windows**. The parts of the client Millennium patches natively
(a few non-web surfaces) stay as Valve draws them, and a theme's own options (the `Statement` conditionals in
its manifest) are applied in their default form rather than rendered as settings. A theme's optional
JavaScript runs only while **Theme scripts** is switched on in Settings.

## Requirements

- Windows 10/11 with the WebView2 runtime (present on virtually every current install)
- Node 20+ and a Rust toolchain
- Either the MSVC toolchain (Visual Studio Build Tools with C++) **or** the GNU toolchain plus MinGW-w64.
  This build was produced with `stable-x86_64-pc-windows-gnu` + WinLibs, because no MSVC linker was
  available on the build machine.

## Build

```bash
npm install

# optional: point the build at your own Worker instead of the shipped one
STEAMWARE_API_BASE=https://steamware-api.you.workers.dev npm run exe
```

`npm run exe` is `tauri build --no-bundle`: one standalone executable in
`src-tauri/target/release/steamware.exe`. Add `--bundle` (or `npm run tauri build`) for an NSIS installer.

`build.rs` bakes in the production Worker (`steamware-api.sparxreaderbypass.workers.dev`) unless
`STEAMWARE_API_BASE` says otherwise, so a release build needs no variables and its first run never asks for an
address. Only a source checkout built for a *different* Worker has to name one. Neither path embeds a key —
there is no key to embed.

If you are on the GNU toolchain, put MinGW on `PATH` before building:

```bash
export PATH="/c/path/to/mingw64/bin:$HOME/.cargo/bin:$PATH"
```

## Develop

```bash
npm run desktop      # vite + tauri dev, hot reload
```

The dev window loads `http://127.0.0.1:1420`. Point Settings at `http://127.0.0.1:8787` while running
`wrangler dev`, and use `POST /v1/session/dev` (loopback-only, development-only) to skip Turnstile.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Covers the Steam parsers (VDF quoted tokens, `appmanifest` fields, library folder discovery). The network
and session logic is covered on the server side, where it lives.

## How it is put together

```
src/                  React UI (design tokens in styles.css)
  assets/             the product mark, the same artwork as the window icon
  lib.ts              typed bridge: every invoke, one place
  components.tsx      shell chrome, tiles, drop zone, toasts, auth overlay
  pages.tsx           the eight pages
src-tauri/src/
  api.rs              the only network client; one host, bearer token, one re-verify retry
  commands.rs         the command surface the UI is allowed to call
  downloads.rs        streaming queue, .part files, progress events, cancellation
  media.rs            cover proxy → data: URLs, disk-cached
  steam.rs            registry detection, libraryfolders.vdf, appmanifest parsing
  settings.rs         validated, atomically-written profile
  error.rs            { code, message } shape the UI branches on
```

A few decisions worth knowing:

- **One mark, everywhere.** `resources/steamware-icon.png` is the artwork behind the window and taskbar
  icon (`icons/icon.ico`), the title bar and the onboarding card (`src/assets`, imported by Vite), the
  icon the plugin asks for through `/icon`, and the `luatools-icon.png` the plugin bundle ships — that file
  is overwritten with it on install. Nothing draws its own placeholder.
- **Frameless window.** The title bar is drawn by the UI (drag region + window controls) to match the WPF
  shell, so the app owns its chrome.
- **Covers as `data:` URLs.** An `<img>` cannot carry the session header; Rust fetches through the proxy and
  hands the UI bytes. Cached in memory and on disk, so a grid costs one round trip.
- **Plugin state is never assumed.** Only the release feed's digest can say the loader is out of date. When
  the feed cannot be read the page says "Installed", withholds the verdict pill and does not offer an
  update it cannot describe.
- **24-hour sessions.** A session stays good for a day; the token lives in memory only, so restarting the
  app asks for a fresh check. That is the trade for nothing sensitive on disk.
- **Downloads never buffer in the webview.** The queue streams to disk in Rust and emits progress events.

## Releasing (the update gate)

Every client asks `https://api.github.com/repos/steamware-dev/steamware/releases/latest` whether it is still
current, and a build that is not refuses to open — the window shows one card and no pages are mounted. Three
things make that work, and all three are the release process:

1. **Bump the version in three places** — `package.json`, `src-tauri/tauri.conf.json` and
   `src-tauri/Cargo.toml` — then build. The version compiled into the binary is what every client compares.
2. **Publish with a `v<version>` tag, not as a pre-release.** `/latest` ignores drafts and pre-releases, so
   those are invisible to the gate. That is how to stage a release, and also how to call one back.
3. **Attach the NSIS setup**, with `setup` or `installer` in its name (`Steamware_<version>_x64-setup.exe` is what
   the build produces). The gate downloads that asset, checks its SHA-256 against GitHub's digest, and runs it
   silently after quitting. Without it the gate still blocks but sends people to the release page; the
   standalone `-regular.exe` / `-tiny.exe` assets are deliberately never used, since running one would put a
   second copy beside the installed app instead of upgrading it.

An unreachable feed never blocks — otherwise an offline launch would lock people out of a working app — but a
feed that was read once and later fails does keep blocking, so blocking GitHub is not a way past the gate.
Taking a release down moves everyone back to the previous version, which is the emergency lever.

## Profile and uninstalling

This build keeps its profile in `%APPDATA%\SteamwareDesktop`, **not** `%APPDATA%\Steamware`. The WPF app
owns the latter and its `settings.json` has a different schema (and holds the user's Hubcap key), so sharing
it would quietly destroy those settings on the first save. Both apps can run side by side.

Settings → About shows the folder. Removing it deletes settings, the device id, the media cache and any
`.part` files; downloads you saved elsewhere are untouched.
