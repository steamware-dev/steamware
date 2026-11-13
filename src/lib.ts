import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

/* ── Types mirroring the Rust structs ───────────────────────────────────────────────────────────── */

export interface Settings {
  apiBase: string;
  accent: string;
  mode: "dark" | "light";
  downloadDir: string;
  steamPath: string;
  maxConcurrent: number;
  showSecurityBanner: boolean;
  completedOnboarding: boolean;
  /** The user's own Hubcap key, typed into Settings and kept only in this profile. */
  hubcapKey: string;
  /** Which source Fetch prefers: whichever can serve the game, the built-in one, or Hubcap. */
  preferredSource: "auto" | "depotbox" | "hubcap";
  /** Launch Steamware when Windows starts. */
  startWithWindows: boolean;
  /** Closing the window hides it to the notification area instead of quitting. */
  minimizeToTray: boolean;
  /** Publish the page, or the game being downloaded, on the user's Discord profile. */
  discordRpc: boolean;
  /** The Discord application whose Rich Presence this profile publishes under. Empty keeps it idle. */
  discordAppId: string;
  /** Install fetched luas with their manifest pins commented out, so Steam keeps games updated. */
  autoUpdateApps: boolean;
  /** Probe a source before downloading, so a game the first one cannot serve comes from the other. */
  fastFetch: boolean;
  /** Hidden marker set the first time the profile is saved. */
  accentChosen?: boolean;
}

/** What Hubcap said about a key: whether it works, and how much of its daily quota is used. */
export interface HubcapCheck {
  ok: boolean;
  message: string;
}

export interface DiscordCheck {
  ok: boolean;
  message: string;
}

export interface SteamStatus {
  installed: boolean;
  running: boolean;
  installPath: string | null;
  libraryCount: number;
  gameCount: number;
  /** Games added through Steamware (one lua each) — the library page's real subject. */
  ownedCount: number;
}

/** A game Steamware has added to Steam: its lua in `config\stplug-in` plus whatever Steam knows on disk. */
export interface OwnedGame {
  appId: string;
  name: string;
  luaPath: string;
  /** Unix seconds the lua was written — "recently added" order. */
  addedAt: number;
  installed: boolean;
  sizeOnDisk: number;
  lastPlayed: number;
  installDir: string;
  library: string;
  depotCount: number;
}

/** What removing a game actually cleared. Steam and Steamware hold separate records for one game. */
export interface RemovalSummary {
  /** The lua: Steamware's library entry and the licence Steam needs to start it. */
  lua: boolean;
  /** The `appmanifest` Steamware wrote: the game's tile in Steam's own library. */
  library: boolean;
  /** Steam has the game on disk, so its record stays — uninstall it from Steam to clear that tile. */
  stillInstalled: boolean;
  /** Steam only re-reads these folders at startup, so a running client keeps the old tile until then. */
  steamRunning: boolean;
}

export interface InstalledGame {
  appId: string;
  name: string;
  installDir: string;
  library: string;
  sizeOnDisk: number;
  lastPlayed: number;
  stateFlags: number;
}

export interface DownloadItem {
  id: string;
  appId: string;
  title: string;
  kind: "manifest" | "fix" | string;
  status: "queued" | "downloading" | "done" | "failed" | "cancelled" | string;
  received: number;
  total: number;
  fileName: string;
  path: string;
  error: string | null;
  startedAt: number;
  /** Lua files this archive put into Steam's plugin folder. */
  installedLua: number;
  /** Manifests this archive put into `depotcache`. */
  installedManifests: number;
  /** Set when the archive landed but Steam refused it (not installed / not found). */
  installError: string | null;
  /** Steam will pick these files up without a restart (the plugin folder is being watched). */
  installLive: boolean;
  /** The game was also written into Steam's library, so it is a tile there now. */
  installLibrary: boolean;
}

/** What an install put into Steam. */
export interface InstallSummary {
  lua: number;
  manifests: number;
  /** The unlocker is watching the plugin folder, so a running Steam loads these by itself. */
  live: boolean;
  steamRunning: boolean;
  /** The game was also handed to Steam's library, so it is a tile there now. */
  library: boolean;
}

/**
 * One file handed to the installer.
 *
 * A picker gives a `path`; a drop gives the bytes as base64, because a webview is handed a `File` with no path
 * on disk and the installer has to read what was actually dragged in.
 */
export interface ImportFile {
  name: string;
  path?: string;
  data?: string;
}

export interface HistoryEntry {
  id: string;
  appId: string;
  title: string;
  kind: string;
  status: string;
  size: number;
  startedAt: number;
  fileName: string;
}

export interface SessionInfo {
  expiresAt: number;
  deviceHash: string;
}

export interface Bootstrap {
  settings: Settings;
  steam: SteamStatus;
  version: string;
  session: SessionInfo | null;
  defaultApiBase: string;
  dataDir: string;
}

export interface StoreItem {
  appid: number;
  name: string;
  cover: string;
}

export interface FixEntry {
  id: string;
  downloadName: string;
  size: string | null;
  badges: string[];
  tags: string[];
}

export interface FixGame {
  appid: string;
  name: string;
  headerImage: string | null;
  capsuleImage: string | null;
  fixes: FixEntry[];
}

export interface FixCatalog {
  ok: boolean;
  count: number;
  availableTags: string[];
  games: FixGame[];
}

export interface BuildGame {
  appId: string;
  name: string;
  hasLua: boolean;
  depotCount: number;
  pinCount: number;
  variantCount: number;
  lastPlayed: number;
}

export interface LuaRow {
  id: string;
  kind: "base" | "depot" | "dlc";
  enabled: boolean;
  canLock: boolean;
  locked: boolean;
  manifest: string | null;
  comment: string | null;
  size: number | null;
}

export interface LuaVariant {
  id: string;
  name: string;
  createdAt: number;
}

export interface LuaState {
  appId: string;
  name: string;
  luaPath: string;
  exists: boolean;
  text: string;
  rows: LuaRow[];
  depotCount: number;
  dlcCount: number;
  pinned: number;
  variants: LuaVariant[];
  activeVariant: string | null;
}

export interface DepotInfo {
  appid: string;
  buildId: string | null;
  dlcIds: string[];
  depots: Array<{
    id: string;
    size: number;
    dlcAppId: string | null;
    shared: boolean;
    os: string | null;
    language: string | null;
    manifestId: string | null;
  }>;
}

export interface ModeCard {
  id: string;
  title: string;
  description: string;
  active: boolean;
  installed: boolean;
  version: string | null;
  latest: string | null;
  updateAvailable: boolean;
  status: string;
  recommended: boolean;
  experimental: boolean;
  managed: boolean;
  /** False on macOS and Linux, where the Windows loader cannot be installed. */
  supported: boolean;
}

export interface CloudState {
  installed: boolean;
  enabled: boolean;
  updateAvailable: boolean;
  unlocked: boolean;
  status: string;
}

export interface ModesState {
  steamPath: string | null;
  cards: ModeCard[];
  cloud: CloudState;
}

export interface ServerConfig {
  version: string;
  sources: Array<{ id: string; label: string; manifests: boolean; fixes: boolean }>;
}

export interface PluginState {
  installed: boolean;
  frontend: boolean;
  loader: boolean;
  dllMatches: boolean;
  /** False when the release feed could not confirm the loader, so `dllMatches` means nothing yet. */
  dllVerified: boolean;
  version: string | null;
  latest: string | null;
  updateAvailable: boolean;
  millennium: boolean;
  /** Steam is running without its plugin port, so store pages need one restart before the plugin appears. */
  restartRequired: boolean;
  steamPath: string | null;
  status: string | null;
}

export interface StoreApp {
  appid: string;
  name: string;
  headerImage: string | null;
  shortDescription: string | null;
  developers: string[];
  genres: string[];
  releaseDate: string | null;
}

/**
 * What a card shows beside its art: Steam's review score, the number of reviews behind it, and how many
 * people are playing right now.
 *
 * Any field can be missing — a game nobody has reviewed has no score — so the card draws whichever of the
 * two pills it actually has instead of a row of dashes. `at` is when the server assembled this answer, which
 * is what the client's minute-long cache is measured against.
 */
export interface GameStat {
  percent: number | null;
  reviews: number;
  players: number | null;
  desc: string;
  at: number;
  /**
   * When Steam was actually read for these numbers.
   *
   * Usually the same moment as `at`. It is older when the Worker served a reading it already had because
   * Valve refused the refresh, which is worth saying out loud — a player count from twenty minutes ago is a
   * real figure, but it is not a live one, and the card must not present it as one.
   */
  readAt?: number;
}

/**
 * Whether the Steamware server is usable right now.
 *
 * When it is not — the free daily budget is spent, or the Worker is answering with errors — the app keeps
 * working from what is already local instead of failing: the library, settings, builds and running downloads
 * are all local, and covers come from Valve's CDN. This is what the banner is drawn from.
 */
export interface ServerHealth {
  degraded: boolean;
  message: string | null;
  /** Unix seconds the hold lifts. */
  until: number;
}

/**
 * Whether this build is still the newest release.
 *
 * `required` is the only field the app branches on, and Rust only sets it when it has read a strictly newer
 * version number. An unreachable feed leaves it false so an offline user is never locked out.
 */
export interface UpdateStatus {
  current: string;
  latest: string;
  required: boolean;
  releaseUrl: string;
  setupUrl: string;
  /** The installer can be fetched and run here, rather than sending the user to GitHub. */
  canInstall: boolean;
  notes: string;
  /** Unix seconds of the last read. */
  checkedAt: number;
  error: string | null;
}

/* ── Commands ─────────────────────────────────────────────────────────────────────────────────── */

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),

  steamRefresh: () => invoke<SteamStatus>("steam_refresh"),
  steamLaunch: () => invoke<void>("steam_launch"),
  libraryList: () => invoke<InstalledGame[]>("library_list"),
  libraryOwned: () => invoke<OwnedGame[]>("library_owned"),
  /**
   * Removes one game from Steamware's library (its lua) and from Steam's own (the manifest Steamware wrote).
   * The game's downloaded files are left alone, and a downloaded game keeps Steam's record.
   */
  libraryRemove: (appId: string) => invoke<RemovalSummary>("library_remove", { appId }),
  steamRestart: () => invoke<void>("steam_restart"),

  authBegin: () => invoke<void>("auth_begin"),
  authSignOut: () => invoke<void>("auth_sign_out"),
  authStatus: () => invoke<SessionInfo | null>("auth_status"),

  /** Local state only: whether the app is holding back optional requests, and until when. */
  serverHealth: () => invoke<ServerHealth>("server_health"),

  /** Cached only: safe to poll. */
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  updateCheck: () => invoke<UpdateStatus>("update_check"),
  updateInstall: () => invoke<void>("update_install"),

  serverConfig: () => invoke<ServerConfig>("server_config"),
  catalogStats: () => invoke<{ games: number; downloads24h: number; fetchedAt: number }>("catalog_stats"),
  storeSearch: (query: string) => invoke<{ items: StoreItem[] }>("store_search", { query }),
  storeFeatured: () =>
    invoke<{ newReleases: StoreItem[]; topSellers: StoreItem[]; specials: StoreItem[] }>("store_featured"),
  storeGenre: (genre: string, start: number) =>
    invoke<{ items: StoreItem[]; total: number; start: number }>("store_genre", { genre, start }),
  storeApp: (appId: string) => invoke<StoreApp>("store_app", { appId }),
  storeCover: (appId: string) => invoke<StoreApp>("store_cover", { appId }),
  storeStats: (appIds: string[]) =>
    invoke<{ stats: Record<string, GameStat | null> }>("store_stats", { appIds }),
  fixesCatalog: (tag: string, query: string) =>
    invoke<FixCatalog>("fixes_catalog", { tag, query }),
  manifestAvailability: (appId: string, source = "depotbox") =>
    invoke<{ available: boolean }>("manifest_availability", { appId, source }),

  startManifestDownload: (appId: string, title: string, source = "depotbox") =>
    invoke<DownloadItem>("start_manifest_download", { appId, title, source }),
  startFixDownload: (id: string, file: string | null, title: string) =>
    invoke<DownloadItem>("start_fix_download", { id, file, title }),

  downloadsList: () => invoke<DownloadItem[]>("downloads_list"),
  downloadCancel: (id: string) => invoke<void>("download_cancel", { id }),
  downloadHistory: () => invoke<HistoryEntry[]>("download_history"),
  downloadHistoryClear: () => invoke<void>("download_history_clear"),
  downloadHistoryRemove: (id: string) => invoke<void>("download_history_remove", { id }),

  openMedia: (url: string) => invoke<string>("open_media", { url }),
  openExternal: (url: string) => invoke<void>("open_external", { url }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  installLua: (path: string) => invoke<InstallSummary>("install_lua_from_zip", { path }),
  /** Installs whatever was dropped or picked: a `.zip`, a `.lua`, a `.manifest`, or a `.ua` holding one. */
  installFiles: (files: ImportFile[]) => invoke<InstallSummary>("install_files", { files }),
  pickArchive: () => invoke<string | null>("pick_archive"),
  /** The drop zone's own picker: `.lua`, `.manifest`, `.ua` and `.zip`, several at once. */
  pickFiles: () => invoke<string[]>("pick_files"),
  pickFolder: () => invoke<string | null>("pick_folder"),

  steamApp: (appId: string) => invoke<DepotInfo>("steam_app", { appId }),

  buildsList: () => invoke<BuildGame[]>("builds_list"),
  buildsState: (appId: string) => invoke<LuaState>("builds_state", { appId }),
  buildsSetEntry: (appId: string, entryId: string, field: "enabled" | "locked", value: boolean) =>
    invoke<LuaState>("builds_set_entry", { appId, entryId, field, value }),
  buildsSaveText: (appId: string, text: string) => invoke<LuaState>("builds_save_text", { appId, text }),
  buildsSaveVariant: (appId: string, name: string) =>
    invoke<LuaState>("builds_save_variant", { appId, name }),
  buildsApplyVariant: (appId: string, variantId: string) =>
    invoke<LuaState>("builds_apply_variant", { appId, variantId }),
  buildsRenameVariant: (appId: string, variantId: string, name: string) =>
    invoke<LuaState>("builds_rename_variant", { appId, variantId, name }),
  buildsDeleteVariant: (appId: string, variantId: string) =>
    invoke<LuaState>("builds_delete_variant", { appId, variantId }),

  modesState: () => invoke<ModesState>("modes_state"),
  modesInstall: (mode: string) => invoke<ModesState>("modes_install", { mode }),
  modesCloudRedirect: (enable: boolean, update = false) =>
    invoke<ModesState>("modes_cloud_redirect", { enable, update }),

  pluginState: () => invoke<PluginState>("plugin_state"),
  pluginInstall: () => invoke<PluginState>("plugin_install"),
  pluginUninstall: () => invoke<void>("plugin_uninstall"),

  windowMinimize: () => invoke<void>("window_minimize"),
  windowToggleMaximize: () => invoke<void>("window_toggle_maximize"),
  windowClose: () => invoke<void>("window_close"),

  /** The page the shell just opened, for Discord Rich Presence. */
  setPresencePage: (page: string) => invoke<void>("set_presence_page", { page }),
  /** Checks a Hubcap key against Hubcap itself; never writes it. */
  hubcapCheck: (key: string) => invoke<HubcapCheck>("hubcap_check", { key }),
  /** Checks a Discord application id against the local client; publishes nothing. */
  discordCheck: (appId: string) => invoke<DiscordCheck>("discord_check", { appId }),
  /** Whether Windows would currently start Steamware. */
  startupStatus: () => invoke<boolean>("startup_status"),
};

/* ── Errors ────────────────────────────────────────────────────────────────────────────────────── */

export interface Failure {
  code: string;
  message: string;
}

export function toFailure(error: unknown): Failure {
  if (error && typeof error === "object" && "message" in error) {
    const candidate = error as { code?: unknown; message?: unknown };
    return {
      code: typeof candidate.code === "string" ? candidate.code : "error",
      message: typeof candidate.message === "string" ? candidate.message : "Something went wrong.",
    };
  }
  if (typeof error === "string") return { code: "error", message: error };
  return { code: "error", message: "Something went wrong." };
}

/* ── Hooks ────────────────────────────────────────────────────────────────────────────────────── */

export function useAsync<T>(
  run: () => Promise<T>,
  deps: unknown[],
  options: { skip?: boolean } = {},
) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<Failure | null>(null);
  const [loading, setLoading] = useState(!options.skip);
  const alive = useRef(true);

  const reload = useCallback(() => {
    setLoading(true);
    run()
      .then((value) => {
        if (alive.current) {
          setData(value);
          setError(null);
        }
      })
      .catch((failure) => {
        if (alive.current) setError(toFailure(failure));
      })
      .finally(() => {
        if (alive.current) setLoading(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    alive.current = true;
    if (!options.skip) reload();
    return () => {
      alive.current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reload, options.skip]);

  return { data, error, loading, reload, setData };
}

const coverCache = new Map<string, string>();
const coverInflight = new Map<string, Promise<string>>();

/**
 * A cover as a data URL.
 *
 * The images arrive through the Worker proxy, which needs the session header the webview cannot attach to
 * an <img>. Rust does the fetch and hands back bytes; this keeps one copy per URL for the life of the
 * window so a grid does not re-request art when you switch pages.
 */
/// URLs whose fetch already came back empty. Remembered for the session so a port of dead art costs one
/// request, not one per render.
const coverDead = new Set<string>();

/**
 * Whether the browser will actually paint this source.
 *
 * A data URL came through the proxy, which already refused anything that was not an image. The only other
 * thing the chain can hand back is the bare CDN URL Rust returns while the session is unverified — and art
 * that is not there (an appid with no header or capsule) would otherwise reach the tile as if it were art and
 * paint a broken-image glyph with the game's name spilling out beside it. So it is loaded once first.
 */
function paints(src: string): Promise<boolean> {
  if (!src || src.startsWith("data:")) return Promise.resolve(Boolean(src));
  return new Promise((resolve) => {
    const probe = new Image();
    probe.onload = () => resolve(true);
    probe.onerror = () => resolve(false);
    probe.src = src;
  });
}

function fetchCover(url: string): Promise<string> {
  const cached = coverCache.get(url);
  if (cached) return Promise.resolve(cached);
  const pending = coverInflight.get(url);
  if (pending) return pending;

  const request = api
    .openMedia(url)
    .then(async (data) => {
      if (data && (await paints(data))) {
        coverCache.set(url, data);
        return data;
      }
      coverDead.add(url);
      return "";
    })
    .catch(() => {
      coverDead.add(url);
      return "";
    })
    .finally(() => coverInflight.delete(url));
  coverInflight.set(url, request);
  return request;
}

export function useCover(url: string | null | undefined): string {
  return useCoverChain(url ? [url] : []);
}

/** Header URLs the store has already named, and the calls still in flight for them. */
const headerHints = new Map<string, string>();
const headerInflight = new Map<string, Promise<string>>();

/**
 * The real header URL for an app, from the store.
 *
 * One request per game for the life of the window: a library of 300 tiles must not ask 300 times when a
 * page shows 24 of them.
 */
function resolveCoverHint(appId: string): Promise<string> {
  const known = headerHints.get(appId);
  if (known !== undefined) return Promise.resolve(known);
  const pending = headerInflight.get(appId);
  if (pending) return pending;

  const request = api
    .storeCover(appId)
    .then((app) => {
      const url = app?.headerImage ?? "";
      headerHints.set(appId, url);
      return url;
    })
    .catch(() => {
      // No store page, or no server: remembered as "nothing" so the tile stops asking.
      headerHints.set(appId, "");
      return "";
    })
    .finally(() => headerInflight.delete(appId));
  headerInflight.set(appId, request);
  return request;
}

/**
 * The first URL in `urls` that resolves to actual art.
 *
 * Tried in order, because Steam does not keep every size for every app: when `header.jpg` 404s the next
 * candidate is fetched instead of leaving an empty box in the grid.
 *
 * `appId` makes that chain complete rather than hopeful. Every guess above is a legacy path
 * (`steam/apps/<id>/header.jpg`); apps published after Steam moved to hashed `store_item_assets` paths
 * have none of them, which is why those tiles stayed blank forever. For those, the game's real header
 * URL is fetched once from the store and used as the last candidate.
 */
export function useCoverChain(urls: string[], appId?: string | number): string {
  const key = urls.join("|");
  const [src, setSrc] = useState(() => {
    for (const url of urls) {
      const hit = coverCache.get(url);
      if (hit) return hit;
      break;
    }
    return "";
  });

  useEffect(() => {
    const candidates = urls.filter((url) => url && !coverDead.has(url));
    let alive = true;

    (async () => {
      for (const url of candidates) {
        const data = await fetchCover(url);
        if (!alive) return;
        if (data) {
          setSrc(data);
          return;
        }
      }

      // Every legacy path is dead. Ask the store which hashed URL this app's art actually lives at.
      if (appId === undefined) return;
      const hint = await resolveCoverHint(String(appId));
      if (!alive || !hint) return;
      const data = await fetchCover(hint);
      if (alive && data) setSrc(data);
    })();

    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, appId]);

  return src;
}

export function useDownloads() {
  const [items, setItems] = useState<DownloadItem[]>([]);

  useEffect(() => {
    let alive = true;
    api.downloadsList().then((list) => alive && setItems(list)).catch(() => {});

    const unlisten = listen<DownloadItem>("download:progress", (event) => {
      setItems((previous) => {
        const next = previous.filter((item) => item.id !== event.payload.id);
        next.push(event.payload);
        return next.sort((a, b) => b.startedAt - a.startedAt);
      });
    });

    return () => {
      alive = false;
      unlisten.then((dispose) => dispose());
    };
  }, []);

  return items;
}

/* ── Store stats ─────────────────────────────────────────────────────────────────────────────── */

/**
 * The review score and player count every card draws, held in one place for the whole app.
 *
 * A tile does not fetch; it subscribes. Each tile asks for its own app id, the ids are collected for a moment
 * and then asked for in one request, so a row of eight games costs one round trip instead of eight and two
 * sections showing the same game share the answer. The Worker answers for the first twelve ids of a request and
 * drops the rest without saying so, so the queue is sent in batches of exactly that size: a batch of twenty was
 * how eight cards in a full row ended up with no numbers at all. Each batch is awaited before the next is sent,
 * so a grid wider than twelve cards still sees every one of them filled in.
 *
 * These numbers move — the player count by the second — so an entry goes stale after a minute, and anything
 * that is on screen is asked for again the moment the user comes back to the window (`useStatsAutoRefresh`).
 */
/// Must match `STATS_MAX_APPIDS` in the Worker (`backend/src/app.ts`). It is a hard cap on the Worker side and
/// an over-long batch is truncated silently, so a client that guesses bigger loses the difference.
const STAT_BATCH = 12;
const STAT_TTL_MS = 60_000;
/// After a failure, cards stay quiet for a while rather than re-asking on every render.
const STAT_RETRY_MS = 30_000;
/// A game the Worker answered nothing for — Valve timed out, or its page carried no numbers — is asked about
/// again after this long instead of staying blank for the rest of the session. Those failures are almost always
/// transient, and the window is long enough that a game that is genuinely empty is not a request loop.
const STAT_MISS_RETRY_MS = 15_000;

const statCache = new Map<string, GameStat>();
const statSubscribers = new Map<string, Set<() => void>>();
const statWaiting = new Set<string>();
/// App id → the moment it may be asked for again after an answer that held nothing.
const statMisses = new Map<string, number>();
let statTimer: ReturnType<typeof setTimeout> | null = null;
let statDueAt = 0;
let statRetryAt = 0;
let statFlushing = false;

function statIsFresh(appId: string) {
  const entry = statCache.get(appId);
  return !!entry && Date.now() - entry.at < STAT_TTL_MS;
}

function statIsMissed(appId: string) {
  const until = statMisses.get(appId);
  if (until === undefined) return false;
  if (Date.now() < until) return true;
  statMisses.delete(appId);
  return false;
}

/// What a card knows right now: the numbers, whether it is still waiting for them, or neither. `loading` is
/// what the ghost placeholders are drawn from — a card that is merely empty must not shimmer for ever.
function readStat(appId: string): { stat: GameStat | null; loading: boolean } {
  const stat = statCache.get(appId) ?? null;
  return { stat, loading: !stat && !statIsMissed(appId) };
}

function publishStat(appId: string) {
  const waiters = statSubscribers.get(appId);
  if (!waiters) return;
  for (const notify of waiters) notify();
}

/// One timer for the whole app. A wake-up that is already due sooner than `delay` is kept, so asking "as soon
/// as possible" during a long retry cannot pull the retry forward, and a retry cannot be pushed back.
function scheduleStats(delay = 40) {
  const dueAt = Date.now() + delay;
  if (statTimer) {
    if (statDueAt <= dueAt) return;
    clearTimeout(statTimer);
  }
  statDueAt = dueAt;
  statTimer = setTimeout(() => {
    statTimer = null;
    void flushStats();
  }, delay);
}

async function flushStats() {
  // The timer can fire again while a batch is in flight; two flushes at once would ask twice for the same cards.
  if (statFlushing) return;
  statFlushing = true;
  try {
    const now = Date.now();
    if (now < statRetryAt) {
      // A failure holds everything back for a moment — and then tries again, rather than waiting for the user
      // to leave the page and come back before any card recovers.
      scheduleStats(statRetryAt - now);
      return;
    }

    const ids = [...statWaiting].filter((id) => !statIsFresh(id) && !statIsMissed(id));
    statWaiting.clear();
    if (ids.length === 0) return;

    for (let index = 0; index < ids.length; index += STAT_BATCH) {
      const chunk = ids.slice(index, index + STAT_BATCH);
      try {
        const answer = await api.storeStats(chunk);
        for (const id of chunk) {
          const entry = answer.stats[id];
          if (entry) {
            statCache.set(id, entry);
            statMisses.delete(id);
          } else {
            // A null is a real answer: this game had nothing to say. Record when to ask again, so the card
            // resolves now (no ghost forever) and comes back if the numbers turn up later.
            statMisses.set(id, Date.now() + STAT_MISS_RETRY_MS);
          }
          publishStat(id);
        }
      } catch {
        statRetryAt = Date.now() + STAT_RETRY_MS;
        for (const id of chunk) if (statSubscribers.has(id)) statWaiting.add(id);
        scheduleStats(STAT_RETRY_MS);
        break;
      }
    }

    // A card whose retry window is still open asks again the moment it closes, without needing a focus or a
    // remount to nudge it.
    let soonest = Number.POSITIVE_INFINITY;
    for (const id of statSubscribers.keys()) {
      const until = statMisses.get(id);
      if (until !== undefined && until > Date.now()) soonest = Math.min(soonest, until);
    }
    // A few milliseconds past the deadline: waking exactly on it can still read as "inside the window" when the
    // clock has not ticked, and that wake-up would be spent with nothing to ask for.
    if (Number.isFinite(soonest)) scheduleStats(Math.max(250, soonest - Date.now() + 50));
  } finally {
    statFlushing = false;
  }
}

function wantStat(appId: string) {
  if (statIsFresh(appId) || statIsMissed(appId)) return;
  statWaiting.add(appId);
  scheduleStats();
}

/// Marks everything on screen stale and asks again — what a return to the page does.
export function refreshGameStats() {
  statRetryAt = 0;
  for (const appId of statSubscribers.keys()) {
    statCache.delete(appId);
    statMisses.delete(appId);
    statWaiting.add(appId);
  }
  scheduleStats();
}

/// The numbers for one card, re-rendering it when they arrive or are refreshed.
export function useGameStat(appId: string | number | null | undefined): {
  stat: GameStat | null;
  loading: boolean;
} {
  const id = appId === null || appId === undefined ? "" : String(appId);
  const [snapshot, setSnapshot] = useState(() => readStat(id));

  useEffect(() => {
    if (!id) {
      setSnapshot({ stat: null, loading: false });
      return;
    }
    let waiters = statSubscribers.get(id);
    if (!waiters) {
      waiters = new Set();
      statSubscribers.set(id, waiters);
    }
    const notify = () => setSnapshot(readStat(id));
    waiters.add(notify);
    notify();
    wantStat(id);
    return () => {
      const current = statSubscribers.get(id);
      if (!current) return;
      current.delete(notify);
      if (current.size === 0) statSubscribers.delete(id);
    };
  }, [id]);

  return snapshot;
}

/**
 * Refreshes the cards that are on screen when the window is focused again.
 *
 * "Going off the page and back" is a blur and a focus for this app, and a return from a game's own page is
 * the same signal raised by hand — both end in `refreshGameStats`.
 */
export function useStatsAutoRefresh() {
  useEffect(() => {
    const refresh = () => {
      if (document.hidden) return;
      refreshGameStats();
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    // A card mounted while the window was in the background asks on mount, so nothing is missed by the
    // listener above having fired before there was anything to refresh.
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, []);
}

/**
 * How many cards fit across a surface, always an even number, held near `targetWidth`.
 *
 * The count is measured rather than left to `auto-fill`, because a grid that grows by one column when the
 * window widens leaves an odd, half-empty row — and a row of eight games that becomes a row of seven and a
 * stray card is exactly what reads as unfinished. Two columns is the floor and `max` is the ceiling.
 *
 * A wider window buys *more cards, not larger ones*: the count rounds *up* to the next even number, so the
 * cards a row could fit are never stretched past their target. A step back is taken when that would leave the
 * cards below four fifths of the target, which is what keeps a row of eight from turning into a row of eight
 * postage stamps.
 */
export function useEvenColumns<T extends HTMLElement>(targetWidth: number, max = 12, gap = 14) {
  // A callback ref, not an object ref. The grid being measured only exists once there are cards to put in it,
  // so on the first paint the element is null — and an effect that ran once, found nothing, and depended only
  // on its constants would never run again. That is exactly why a fullscreen store used to sit at the hook's
  // starting four columns: four cards stretched across the whole window, with no resize able to fix it.
  const [element, setElement] = useState<T | null>(null);
  const [cols, setCols] = useState(4);

  useEffect(() => {
    if (!element) return;

    const measure = () => {
      const width = element.clientWidth;
      if (width <= 0) return;
      // The card that would fit whole, as a fraction: 6.5 means six fit comfortably and a seventh is close.
      const wanted = (width + gap) / (targetWidth + gap);

      let even = Math.ceil(wanted / 2) * 2;
      if (even > 2 && (width - (even - 1) * gap) / even < targetWidth * 0.8) even -= 2;

      const next = Math.max(2, Math.min(max, even));
      setCols((previous) => (previous === next ? previous : next));
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element, targetWidth, max, gap]);

  return { ref: setElement, cols };
}

/* ── Formatting ──────────────────────────────────────────────────────────────────────────────── */

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatCount(value: number): string {
  return Number.isFinite(value) ? value.toLocaleString("en-US") : "—";
}

/**
 * A large count in the shape the cards show it: `9.95 M` rather than `9,950,000`.
 *
 * Trailing zeros are dropped so ten million reads as `10 M` and not `10.00 M`, and anything under a thousand
 * keeps its exact value — a review count of 412 is information, not noise.
 */
export function formatCompact(value: number): string {
  if (!Number.isFinite(value)) return "—";
  const trim = (text: string) => text.replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
  if (value >= 1_000_000) return `${trim((value / 1_000_000).toFixed(2))} M`;
  if (value >= 1_000) return `${trim((value / 1_000).toFixed(1))} K`;
  return value.toLocaleString("en-US");
}

export function timeAgo(seconds: number): string {
  if (!seconds) return "never";
  const delta = Math.floor(Date.now() / 1000) - seconds;
  if (delta < 60) return "just now";
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`;
  if (delta < 604800) return `${Math.floor(delta / 86400)}d ago`;
  return new Date(seconds * 1000).toLocaleDateString();
}

export function remaining(seconds: number): string {
  if (seconds <= 0) return "expired";
  // A day-long session rendered as "1440m" is technically true and unreadable; hours start at one hour.
  const hours = Math.floor(seconds / 3600);
  if (hours > 0) {
    const minutes = Math.floor((seconds % 3600) / 60);
    return `${hours}h ${minutes}m`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes > 0 ? `${minutes}m ${rest}s` : `${rest}s`;
}

export const steamHeader = (appId: string | number) =>
  `https://cdn.akamai.steamstatic.com/steam/apps/${appId}/header.jpg`;

/**
 * The two hostnames that serve the same Steam art.
 *
 * They are interchangeable mirrors, with one asymmetry that matters: Akamai answers the media proxy with
 * 502 for the big `library_hero.jpg` files while the Cloudflare name serves the identical bytes, so the
 * wide art is asked for on the Cloudflare host first and Akamai is the fallback.
 */
const ART_HOSTS = ["cdn.cloudflare.steamstatic.com", "cdn.akamai.steamstatic.com"] as const;

const artOn = (host: string, appId: string | number, file: string) =>
  `https://${host}/steam/apps/${appId}/${file}`;

/** The 1920×620 banner Steam paints across a game's own library page. */
export const steamLibraryHero = (appId: string | number) => artOn(ART_HOSTS[0], appId, "library_hero.jpg");

export const steamCapsule = (appId: string | number) =>
  `https://cdn.akamai.steamstatic.com/steam/apps/${appId}/capsule_231x87.jpg`;

export const steamWideCapsule = (appId: string | number) =>
  `https://cdn.akamai.steamstatic.com/steam/apps/${appId}/capsule_616x353.jpg`;

/**
 * Every URL that might hold art for an app, best-looking first.
 *
 * One URL is not enough: `header.jpg` is missing for a lot of games published after Steam moved to hashed
 * asset paths (`…/apps/<id>/<hash>/capsule_sm_120.jpg`), which is exactly the tile that used to come up
 * blank. The wide capsule keeps the tile's own 460:215 shape, and the small capsule always exists.
 */
export function coverCandidates(appId: string | number, hint?: string | null): string[] {
  const id = String(appId);
  const seen = new Set<string>();
  for (const url of [hint ?? "", steamHeader(id), steamWideCapsule(id), steamCapsule(id)]) {
    if (url) seen.add(url);
  }
  return [...seen];
}

/**
 * The wide art for a game's own page, sharpest first.
 *
 * `header.jpg` is 460×215 — stretched across a full-width hero on a maximised window it turns to mush, so
 * it is the fallback, not the first choice. `library_hero.jpg` is 1920×620, the same banner the Steam
 * client paints across a game's library page, and `capsule_616x353` is the widest art every published game
 * still has; between them the hero is real art rather than an upscaled thumbnail at any window size.
 */
export function heroCandidates(appId: string | number, hint?: string | null): string[] {
  const id = String(appId);
  const seen = new Set<string>();
  for (const url of [
    artOn(ART_HOSTS[0], id, "library_hero.jpg"),
    artOn(ART_HOSTS[1], id, "library_hero.jpg"),
    hint ?? "",
    steamHeader(id),
    artOn(ART_HOSTS[0], id, "capsule_616x353.jpg"),
    artOn(ART_HOSTS[1], id, "capsule_616x353.jpg"),
    steamWideCapsule(id),
    steamCapsule(id),
  ]) {
    if (url) seen.add(url);
  }
  return [...seen];
}

/* ── Theme ─────────────────────────────────────────────────────────────────────────────────────── */

/**
 * The accent gallery, ported from `ThemeService.Themes`: the same ids, names and base hexes, with
 * Steamware's own pink leading.
 *
 * The plugin paints itself from whichever accent is chosen (see `skin.rs`), so this swatch is what restores
 * the original pink→purple chrome inside Steam instead of the violet-leaning gradient any other accent
 * produces.
 */
export const PALETTES: Array<{ id: string; name: string; hex: string }> = [
  { id: "brand", name: "Steamware", hex: "#d63b96" },
  { id: "violet", name: "Violet", hex: "#a78bfa" },
  { id: "orchid", name: "Orchid", hex: "#d188dd" },
  { id: "blossom", name: "Blossom", hex: "#e8b0d3" },
  { id: "rose", name: "Rose", hex: "#dd8894" },
  { id: "clay", name: "Clay", hex: "#e7c2ac" },
  { id: "olive", name: "Olive", hex: "#d3c569" },
  { id: "lime", name: "Lime", hex: "#b3d775" },
  { id: "fern", name: "Fern", hex: "#77d369" },
  { id: "mint", name: "Mint", hex: "#88dda7" },
  { id: "teal", name: "Teal", hex: "#69d3c5" },
  { id: "sky", name: "Sky", hex: "#a8cfe6" },
  { id: "indigo", name: "Indigo", hex: "#8894dd" },
  /*
   * The wheel, all of it, drawn soft.
   *
   * The twelve above are the WPF build's gallery and skip whole stretches of the spectrum — there is no red,
   * no orange and no true yellow in it. These fill it in at even thirty-degree steps, and they are deliberately
   * muted rather than vivid: a saturated hue fights the panel it is painted on and glares on white text, while
   * these sit at a saturation the rest of the interface can live with. Every one of them still drives the same
   * derived palette (washes, gradients, the in-Steam panel), so the choice is a colour, not a different look.
   */
  { id: "coral", name: "Coral", hex: "#da9f8b" },
  { id: "sand", name: "Sand", hex: "#dac68b" },
  { id: "pistachio", name: "Pistachio", hex: "#c6da8b" },
  { id: "meadow", name: "Meadow", hex: "#9fda8b" },
  { id: "jade", name: "Jade", hex: "#8bda9f" },
  { id: "sea", name: "Sea", hex: "#8bdac6" },
  { id: "aqua", name: "Aqua", hex: "#8bc6da" },
  { id: "cornflower", name: "Cornflower", hex: "#8b9fda" },
  { id: "periwinkle", name: "Periwinkle", hex: "#9f8bda" },
  { id: "orris", name: "Orris", hex: "#c68bda" },
  { id: "peony", name: "Peony", hex: "#da8bc6" },
  { id: "blush", name: "Blush", hex: "#da8b9f" },
];

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

function toHsl(hex: string): [number, number, number] {
  const value = hex.replace("#", "");
  const r = parseInt(value.slice(0, 2), 16) / 255;
  const g = parseInt(value.slice(2, 4), 16) / 255;
  const b = parseInt(value.slice(4, 6), 16) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d < 1e-9) return [0, 0, l];
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
}

function fromHsl(hue: number, saturation: number, lightness: number): [number, number, number] {
  const h = ((hue % 360) + 360) % 360;
  const c = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = lightness - c / 2;
  const [r, g, b] =
    h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return [Math.round((r + m) * 255), Math.round((g + m) * 255), Math.round((b + m) * 255)];
}

const rgb = ([r, g, b]: [number, number, number]) => `rgb(${r}, ${g}, ${b})`;
const alpha = ([r, g, b]: [number, number, number], a: number) => `rgba(${r}, ${g}, ${b}, ${a})`;
/** `AccentPalette.For` — one base accent in, the soft/muted/deep run and the wash steps out. */
export function accentColors(hex: string) {
  const [hue, rawSaturation, rawLightness] = toHsl(hex);
  const saturation = clamp(rawSaturation, 0.2, 1);
  const lightness = clamp(rawLightness, 0.62, 0.8);
  const base = fromHsl(hue, saturation, lightness);
  const soft = fromHsl(hue, Math.min(1, saturation + 0.03), Math.min(0.9, lightness + 0.1));
  const muted = fromHsl(hue, saturation * 0.7, clamp(lightness - 0.03, 0.6, 0.8));
  const deep = fromHsl(hue, Math.min(0.85, saturation), clamp(lightness - 0.18, 0.35, 0.6));
  const surface = fromHsl(hue, 0.3, 0.2);
  return { hue, accent: base, soft, muted, deep, surface };
}

/**
 * Paint the window from one accent, the way `ThemeService` does it.
 *
 * Every surface is the accent at a low alpha rather than white at a low alpha: that is what makes the page
 * read as "the app is lime" instead of "the app is grey with a lime button". The page itself stays neutral;
 * only the ladder one step in is tinted.
 */
export function applyTheme(accent: string, mode: "dark" | "light") {
  const root = document.documentElement;
  const light = mode === "light";
  root.dataset.theme = mode;

  const { hue, accent: base, soft, muted, deep, surface } = accentColors(accent);
  const lift = light ? deep : base;

  const set = (name: string, value: string) => root.style.setProperty(name, value);
  set("--accent", rgb(base));
  // The gallery runs from pale blossom to fern, so a fixed white label on the accent disappears on the light
  // end of it. Pick whichever ink has the better WCAG contrast against this palette's accent.
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  };
  const [inkR, inkG, inkB] = base;
  const luminance = 0.2126 * channel(inkR) + 0.7152 * channel(inkG) + 0.0722 * channel(inkB);
  const onLight = (luminance + 0.05) / 0.05;
  const onDark = 1.05 / (luminance + 0.05);
  set("--accent-ink", onLight >= onDark ? "#14161c" : "#ffffff");
  set("--accent-bright", rgb(soft));
  set("--accent-deep", rgb(deep));
  set("--accent-muted", rgb(muted));
  set("--accent-surface", rgb(surface));
  set("--accent-wash", alpha(base, 0x1a / 255));
  set("--accent-wash-strong", alpha(base, 0x40 / 255));
  set("--accent-line", alpha(base, 0x40 / 255));

  set("--surface-subtle", alpha(lift, (light ? 0x08 : 0x0a) / 255));
  set("--surface", alpha(lift, (light ? 0x0d : 0x14) / 255));
  set("--surface-raised", alpha(lift, (light ? 0x14 : 0x1f) / 255));
  set("--surface-strong", alpha(lift, (light ? 0x1f : 0x2e) / 255));
  set("--hairline", alpha(lift, (light ? 0x0c : 0x10) / 255));
  set("--stroke", alpha(lift, (light ? 0x1c : 0x26) / 255));

  set("--panel", rgb(light ? fromHsl(hue, 0.16, 0.985) : fromHsl(hue, 0.22, 0.145)));
  set("--panel-deep", rgb(light ? fromHsl(hue, 0.18, 0.925) : fromHsl(hue, 0.25, 0.075)));
  set("--page", light ? "#f4f5f9" : "#17181e");

  set("--text", light ? "#1f2430" : "#e5e7eb");
  set("--text-strong", light ? "#2b3240" : "#d1d5db");
  set("--text-2", light ? "#475069" : "#9ca3af");
  set("--muted", light ? "#5b6472" : "#6b7280");
  set("--disabled", light ? "#9ca3af" : "#4b5563");
  set("--scrim", light ? "rgba(15, 18, 28, 0.45)" : "rgba(0, 0, 0, 0.6)");

  // Native widgets (scrollbars, select popups) follow the page, not the palette.
  root.style.colorScheme = mode;
}
