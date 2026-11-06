import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ArrowRight,
  Check,
  Eye,
  EyeOff,
  FolderOpen,
  Gamepad2,
  Layers,
  Library,
  Loader2,
  MessageCircle,
  RefreshCw,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import {
  api,
  applyTheme,
  PALETTES,
  formatBytes,
  formatCount,
  remaining,
  coverCandidates,
  steamHeader,
  toFailure,
  useAsync,
  useCoverChain,
  useEvenColumns,
  refreshGameStats,
  useStatsAutoRefresh,
  type Bootstrap,
  type DownloadItem,
  type FixCatalog,
  type FixGame,
  type OwnedGame,
  type HubcapCheck,
  type ServerConfig,
  type SessionInfo,
  type Settings,
  type SteamStatus,
} from "./lib";
import {
  BrandMark,
  DiscordPill,
  DownloadRow,
  Empty,
  FileDropZone,
  FixCard,
  ArtTile,
  PageHead,
  SearchField,
  SkeletonGrid,
  Toggle,
  type Route,
  type ToastKind,
} from "./components";
import { GameMenu } from "./gameMenu";

/** The little bit of app-level capability every page needs. */
export interface PageApi {
  notify: (kind: ToastKind, title: string, detail?: string) => void;
  run: <T>(task: () => Promise<T>, success?: string) => Promise<T | null>;
  beginAuth: () => void;
  authenticated: boolean;
  session: SessionInfo | null;
  version: string;
}

/// A shelf cover. The URL is a CDN link but the bytes come through the Worker's media proxy: an <img> cannot
/// carry the session header, so the source is a `data:` URL handed back by Rust.
/** `v1.0.3` once, whatever the version string arrives as. The plugin reports its own tag already prefixed and
 *  the app's version has no prefix, so prefixing blindly is what printed "vv1.0.3". */
function versionTag(version: string): string {
  return version.startsWith("v") ? version : `v${version}`;
}

function ShelfTile({ appId, title, onClick }: { appId: string; title: string; onClick: () => void }) {
  const cover = useCoverChain(coverCandidates(appId), appId);
  return (
    <button className="shelf-item" title={title} onClick={onClick}>
      {/* `alt=""`: the button already carries the title, and a name in the slot of missing art is what used
          to spill out of a broken tile. The placeholder says the game's name instead of leaving a hole. */}
      {cover ? (
        <img src={cover} alt="" loading="lazy" />
      ) : (
        <span className="shelf-empty">
          <span className="shelf-empty-name">{title}</span>
        </span>
      )}
    </button>
  );
}

function VerifyCard({ action, title, detail }: { action: () => void; title: string; detail: string }) {
  return (
    <div className="card pad">
      <div style={{ display: "flex", gap: 12, alignItems: "flex-start" }}>
        <ShieldCheck size={18} style={{ color: "var(--accent-bright)", marginTop: 1 }} />
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 500, marginBottom: 3 }}>{title}</div>
          <div className="section-note" style={{ lineHeight: 1.55 }}>
            {detail}
          </div>
        </div>
        <button className="btn primary" onClick={action}>
          Verify
          <ArrowRight size={13} />
        </button>
      </div>
    </div>
  );
}

/* ── Home ─────────────────────────────────────────────────────────────────────────────────────── */

export function HomePage({
  app,
  boot,
  steam,
  downloads,
  onNavigate,
  onRefreshSteam,
}: {
  app: PageApi;
  boot: Bootstrap;
  steam: SteamStatus;
  downloads: DownloadItem[];
  onNavigate: (route: Route) => void;
  onRefreshSteam: () => void;
}) {
  const library = useAsync(() => api.libraryList(), [steam.gameCount, steam.installPath]);
  const stats = useAsync(() => api.catalogStats(), [app.authenticated], { skip: !app.authenticated });
  const featured = useAsync(() => api.storeFeatured(), [app.authenticated], { skip: !app.authenticated });
  const history = useAsync(() => api.downloadHistory(), [downloads.length]);
  const recorded = history.data ?? [];

  const recent = (library.data ?? []).filter((game) => (game.stateFlags & 4) !== 0).slice(0, 9);

  const usage = useMemo(() => {
    const days: Array<{ label: string; count: number }> = [];
    const now = new Date();
    for (let offset = 6; offset >= 0; offset -= 1) {
      const day = new Date(now);
      day.setDate(now.getDate() - offset);
      const key = day.toDateString();
      const count = recorded.filter(
        (item) => new Date(item.startedAt * 1000).toDateString() === key,
      ).length;
      days.push({ label: day.toLocaleDateString(undefined, { weekday: "short" }), count });
    }
    return days;
  }, [recorded]);

  const peak = Math.max(1, ...usage.map((day) => day.count));
  const totalWeek = usage.reduce((sum, day) => sum + day.count, 0);

  const plugin = useAsync(() => api.pluginState(), []);
  const modes = useAsync(() => api.modesState(), []);
  const pluginInstalled = plugin.data?.installed ?? false;
  const activeMode = modes.data?.cards.find((card) => card.active)?.title;

  // "Recently added" is the download history's shelf, not the library's: the old Home listed the games you
  // pulled in, with no captions, one row of nine. When there is no history yet it falls back to what you
  // actually played, so a fresh profile is not an empty band.
  const shelf = useMemo(() => {
    const seen = new Set<string>();
    const items: Array<{ appId: string; title: string }> = [];
    for (const item of recorded) {
      if (item.status !== "done" || !item.appId || seen.has(item.appId)) continue;
      seen.add(item.appId);
      items.push({ appId: item.appId, title: item.title });
    }
    if (items.length >= 3) return items.slice(0, 9);
    return recent.map((game) => ({ appId: game.appId, title: game.name })).slice(0, 9);
  }, [recorded, recent]);

  const picks = useMemo(
    () => [
      ...(featured.data?.specials ?? []),
      ...(featured.data?.newReleases ?? []),
      ...(featured.data?.topSellers ?? []),
    ],
    [featured.data],
  );
  const [pick, setPick] = useState(0);
  const chosen = picks.length > 0 ? picks[pick % picks.length] : null;
  const pickCover = useCoverChain(chosen ? coverCandidates(chosen.appid, chosen.cover) : []);

  return (
    <div className="page-inner home">
      <div className="home-head">
        <h1>Welcome to Steamware</h1>
        <DiscordPill onClick={() => void api.openExternal("https://discord.gg/steamtools")} />
      </div>

      <div className="home-tiles">
        <button className="home-stat" onClick={() => onNavigate("library")}>
          <div className="home-stat-value accent">{formatCount(steam.ownedCount)}</div>
          <div className="home-stat-caption">
            {steam.installed ? "Games you own!" : "No Steam library yet"}
          </div>
        </button>
        <button className="home-stat" onClick={() => onNavigate("store")}>
          <div className="home-stat-value info">
            {stats.data ? formatCount(stats.data.games) : app.authenticated ? "…" : "—"}
          </div>
          <div className="home-stat-caption">Game database</div>
        </button>
        <button className="home-stat plugin" onClick={() => onNavigate("plugin")}>
          <div className="home-stat-label">Plugin Status</div>
          <div className={`home-stat-status ${pluginInstalled ? "ok" : "warn"}`}>
            {pluginInstalled ? `Installed · ${versionTag(plugin.data?.version ?? boot.version)}` : "Not installed"}
          </div>
        </button>
      </div>

      <div className="home-status card">
        <div className="status-line" onClick={() => onNavigate("settings")}>
          <Gamepad2 size={16} style={{ color: "var(--ok)" }} />
          <span className="status-text">
            {steam.installed ? (
              <>
                Steam detected · <span className="status-dim">{steam.installPath}</span>
              </>
            ) : (
              "Steam was not found — set its folder in Settings"
            )}
          </span>
          <button className="icon-btn" onClick={() => onRefreshSteam()} title="Rescan Steam">
            <RefreshCw size={13} />
          </button>
        </div>
        <div className="status-line" onClick={() => onNavigate("mode")}>
          <SlidersHorizontal size={16} style={{ color: "var(--text-2)" }} />
          <span className="status-text">
            Mode · {activeMode ?? "nothing active"}
          </span>
        </div>
        <div className="status-line">
          <ShieldCheck size={16} style={{ color: app.authenticated ? "var(--ok)" : "var(--warn)" }} />
          <span className="status-text">
            {app.authenticated
              ? `Verified · session ends in ${remaining((app.session?.expiresAt ?? 0) - Math.floor(Date.now() / 1000))}`
              : "Not verified — a quick Cloudflare check unlocks manifests and fixes"}
          </span>
          {!app.authenticated && (
            <button className="btn small primary" onClick={app.beginAuth}>
              Verify
            </button>
          )}
        </div>
      </div>

      <FileDropZone
        onDone={(summary) =>
          app.notify(
            "ok",
            "Added to Steam",
            `${summary.lua} lua · ${summary.manifests} manifest${summary.manifests === 1 ? "" : "s"}${
              summary.library ? " · in your library" : ""
            } — Steam waits for you to press Install, it does not download on its own.`,
          )
        }
      />

      {shelf.length > 0 && (
        <section className="shelf">
          <h2>Recently added</h2>
          <div className="shelf-row">
            {shelf.map((item) => (
              <ShelfTile
                key={item.appId}
                appId={item.appId}
                title={item.title}
                onClick={() => onNavigate("library")}
              />
            ))}
          </div>
        </section>
      )}

      <div className="home-bottom">
        <div className="card pad usage-card">
          <div className="usage-head">
            <div className="usage-title">Download usage</div>
            <div className="usage-sub">games added per day · last 7 days</div>
          </div>
          <div className="chart">
            {usage.map((day, index) => (
              <div
                key={index}
                className={`bar ${day.count > 0 ? "hot" : ""}`}
                style={{ height: `${Math.max(4, (day.count / peak) * 100)}%` }}
                title={`${day.label}: ${day.count}`}
              />
            ))}
          </div>
          <div className="usage-foot">{totalWeek} added in the last 7 days</div>
        </div>

        <div className="card pad picks-card">
          <div className="picks-head">
            <span>Random picks</span>
            <button
              className="icon-btn"
              title="Show another"
              onClick={() => {
                if (picks.length > 0) setPick((current) => current + 1);
                else featured.reload();
              }}
            >
              <RefreshCw size={13} />
            </button>
          </div>
          {chosen ? (
            <button className="pick" onClick={() => onNavigate("store")}>
              {pickCover ? <img src={pickCover} alt="" loading="lazy" /> : <span className="pick-empty" />}
              <span className="pick-text">
                <span className="pick-name">{chosen.name}</span>
                <span className="pick-id">App ID: {chosen.appid}</span>
              </span>
            </button>
          ) : (
            <div className="section-note" style={{ padding: "10px 0 4px" }}>
              {app.authenticated ? "Loading the store…" : "Verify to see what everyone is downloading."}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/* ── Store ────────────────────────────────────────────────────────────────────────────────────── */

/* ── Library ───────────────────────────────────────────────────────────────────────────────────── */

const LIB_PAGE_SIZE = 24;

type LibSort = "added" | "name" | "appid" | "played" | "size";
type LibOnly = "all" | "installed" | "missing";

const LIB_SORTS: Array<{ id: LibSort; label: string }> = [
  { id: "added", label: "Recently added" },
  { id: "name", label: "Name (A–Z)" },
  { id: "appid", label: "App ID" },
  { id: "played", label: "Last played" },
  { id: "size", label: "Size on disk" },
];

const LIB_ONLY: Array<{ id: LibOnly; label: string }> = [
  { id: "all", label: "Any" },
  { id: "installed", label: "Downloaded" },
  { id: "missing", label: "Not downloaded" },
];

function OwnedCard({ game, onClick }: { game: OwnedGame; onClick: () => void }) {
  const meta = [game.installed ? formatBytes(game.sizeOnDisk) : null, `${game.depotCount} depots`]
    .filter(Boolean)
    .join(" · ");
  return (
    <ArtTile
      appId={game.appId}
      name={game.name}
      badge={game.installed ? "Installed" : undefined}
      sub={meta}
      onClick={onClick}
    />
  );
}

/**
 * Everything Steamware has added to Steam.
 *
 * The list comes from `config\stplug-in\*.lua`, not from Steam's `appmanifest_*.acf` files. Those only exist
 * for games Steam has downloaded, which is why a library built on them shows 33 rows when the folder holds
 * 312 luas. Installation is a badge here, not a filter on membership.
 */
/// The library's grid uses the store's own card, so it uses the store's own column rule: even counts, two
/// columns in a narrow window and six at most, where a seventh would squeeze the name and the stats together.
/// Same contract as the store grid: a card is asked to be one size and the window buys more of them, not
/// bigger ones. An installed game's card carries its name, its size and its numbers, so it must not shrink
/// into a thumbnail either — hence the step-back inside `useEvenColumns`.
const LIB_GRID_TARGET_CARD = 200;
const LIB_GRID_MAX_COLUMNS = 16;
const LIB_GRID_GAP = 14;

export function LibraryPage({
  app,
  steam,
  onRefresh,
  onManageBuild,
}: {
  app: PageApi;
  steam: SteamStatus;
  onRefresh: () => void;
  onManageBuild?: (appId: string) => void;
}) {
  const { ref: gridRef, cols } = useEvenColumns<HTMLDivElement>(
    LIB_GRID_TARGET_CARD,
    LIB_GRID_MAX_COLUMNS,
    LIB_GRID_GAP,
  );
  // Same contract as the store: the numbers on these cards are re-read when the page is looked at again.
  useStatsAutoRefresh();
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState(false);
  const [sort, setSort] = useState<LibSort>("added");
  const [only, setOnly] = useState<LibOnly>("all");
  const [page, setPage] = useState(1);
  const [perPage, setPerPage] = useState(LIB_PAGE_SIZE);
  const [selected, setSelected] = useState<OwnedGame | null>(null);
  const owned = useAsync(() => api.libraryOwned(), [steam.ownedCount, steam.installPath]);

  const all = useMemo(() => owned.data ?? [], [owned.data]);

  const games = useMemo(() => {
    const needle = query.trim().toLowerCase();
    let list = all;
    if (only === "installed") list = list.filter((game) => game.installed);
    if (only === "missing") list = list.filter((game) => !game.installed);
    if (needle) {
      list = list.filter(
        (game) => game.name.toLowerCase().includes(needle) || game.appId.includes(needle),
      );
    }
    return [...list].sort((a, b) => {
      if (sort === "name") return a.name.localeCompare(b.name);
      if (sort === "appid") return Number(a.appId) - Number(b.appId);
      if (sort === "played") return b.lastPlayed - a.lastPlayed;
      if (sort === "size") return b.sizeOnDisk - a.sizeOnDisk;
      return b.addedAt - a.addedAt;
    });
  }, [all, query, sort, only]);

  const pageCount = Math.max(1, Math.ceil(games.length / perPage));
  const current = Math.min(page, pageCount);
  const shown = games.slice((current - 1) * perPage, current * perPage);

  useEffect(() => setPage(1), [query, sort, only, perPage]);

  // Closing a game, or moving a page, is the moment the cards on screen change — repaint them with numbers
  // read now rather than the ones the previous page was left holding.
  useEffect(() => {
    if (!selected) refreshGameStats();
  }, [selected, current]);

  const installed = all.filter((game) => game.installed).length;

  return (
    <div className="page-inner lib-page">
      <div className="home-head">
        <h1>Library</h1>
        <div className="head-actions">
          <button className="btn ghost" onClick={onRefresh}>
            <RefreshCw size={13} />
            Refresh
          </button>
          <button className={`btn ${filters ? "primary" : "ghost"}`} onClick={() => setFilters((open) => !open)}>
            <SlidersHorizontal size={13} />
            Filters
          </button>
        </div>
      </div>

      <div className="lib-summary">
        {owned.loading && !owned.data ? (
          <span className="section-note">Reading Steam&apos;s plugin folder…</span>
        ) : (
          <span className="section-note">
            {formatCount(all.length)} game{all.length === 1 ? "" : "s"} in your library ·{" "}
            {formatCount(installed)} downloaded
            {query || only !== "all" ? ` · ${formatCount(games.length)} match${games.length === 1 ? "" : "es"}` : ""}
          </span>
        )}
      </div>

      <SearchField value={query} onChange={setQuery} placeholder="Search by name or App ID…" />

      {filters && (
        <div className="card pad lib-filters">
          <div className="lib-filter-row">
            <label className="lib-field">
              <span>Show</span>
              <select value={only} onChange={(event) => setOnly(event.target.value as LibOnly)}>
                {LIB_ONLY.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="lib-field">
              <span>Sort by</span>
              <select value={sort} onChange={(event) => setSort(event.target.value as LibSort)}>
                {LIB_SORTS.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="lib-field">
              <span>Per page</span>
              <select value={perPage} onChange={(event) => setPerPage(Number(event.target.value))}>
                <option value={12}>12</option>
                <option value={24}>24</option>
                <option value={48}>48</option>
                <option value={96}>96</option>
              </select>
            </label>
          </div>
          {(only !== "all" || sort !== "added") && (
            <button
              className="btn ghost small"
              onClick={() => {
                setOnly("all");
                setSort("added");
              }}
            >
              Reset filters
            </button>
          )}
        </div>
      )}

      {!steam.installed ? (
        <Empty
          icon={<Library />}
          title="No Steam library found"
          detail="Steamware reads the luas Steam loads. If Steam lives somewhere unusual, point at it in Settings."
        />
      ) : owned.loading && !owned.data ? (
        <SkeletonGrid count={12} capsule />
      ) : games.length === 0 ? (
        <Empty
          icon={<Library />}
          title={all.length === 0 ? "Nothing here yet" : "No games match"}
          detail={
            all.length === 0
              ? "Games you fetch or drop in are installed into Steam and listed here."
              : "Try another search, or reset the filters."
          }
        />
      ) : (
        <div
          className="store-grid lib-owned-grid"
          ref={gridRef}
          style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
        >
          {shown.map((game) => (
            <OwnedCard key={game.appId} game={game} onClick={() => setSelected(game)} />
          ))}
        </div>
      )}

      {games.length > 0 && (
        <div className="pager">
          <button
            className="icon-btn"
            disabled={current <= 1}
            onClick={() => setPage(Math.max(1, current - 1))}
            aria-label="Previous page"
          >
            ‹
          </button>
          {Array.from({ length: Math.min(pageCount, 5) }).map((_, index) => {
            const number = index + 1;
            return (
              <button
                key={number}
                className={`page-no ${number === current ? "active" : ""}`}
                onClick={() => setPage(number)}
              >
                {number}
              </button>
            );
          })}
          {pageCount > 5 && <span className="page-gap">…</span>}
          <button
            className="icon-btn"
            disabled={current >= pageCount}
            onClick={() => setPage(Math.min(pageCount, current + 1))}
            aria-label="Next page"
          >
            ›
          </button>
          <div className="grow" />
          <span className="section-note">
            Page {current} of {pageCount}
          </span>
        </div>
      )}

      {selected && (
        <GameMenu
          game={selected}
          app={app}
          onClose={() => setSelected(null)}
          onManageBuild={() => {
            const appId = selected.appId;
            setSelected(null);
            onManageBuild?.(appId);
          }}
          onRemoved={() => {
            setSelected(null);
            owned.reload();
          }}
        />
      )}
    </div>
  );
}

/* ── Fixes ───────────────────────────────────────────────────────────────────────────────────── */

const FIX_TAGS = [
  { id: "", label: "All" },
  { id: "online", label: "Online" },
  { id: "bypass", label: "Bypass" },
  { id: "hypervisor", label: "Hypervisor" },
];

export function FixesPage({
  app,
  focus,
}: {
  app: PageApi;
  focus?: { appId: string; at: number } | null;
}) {
  const [tag, setTag] = useState("");
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 320);
    return () => clearTimeout(timer);
  }, [query]);

  // "Fix" on a Steam store page opens this page already filtered to that game.
  useEffect(() => {
    if (!focus?.appId) return;
    setTag("");
    setQuery(focus.appId);
    setDebounced(focus.appId);
  }, [focus?.at, focus?.appId]);

  const catalog = useAsync<FixCatalog>(() => api.fixesCatalog(tag, debounced), [tag, debounced, app.authenticated], {
    skip: !app.authenticated,
  });
  // The server reports whether it holds a provider key. Unknown (offline) is treated as "ready" so this
  // never hides a working catalog behind a failed config call.
  const config = useAsync(() => api.serverConfig(), []);
  const providerReady = config.data?.sources.find((source) => source.id === "depotbox")?.fixes;

  const games = catalog.data?.games ?? [];

  return (
    <div className="page-inner">
      <PageHead
        title="Fixes"
        subtitle={
          catalog.data
            ? `${formatCount(catalog.data.count)} games with a fix · refreshed live, nothing cached`
            : "Online fixes and bypasses for Steam games"
        }
        actions={
          <button className="btn ghost" onClick={() => catalog.reload()}>
            <RefreshCw size={13} />
            Refresh
          </button>
        }
      />

      <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
        <SearchField value={query} onChange={setQuery} placeholder="Search by game name or AppID…" />
        <div className="chips">
          {FIX_TAGS.map((entry) => (
            <button
              key={entry.id}
              className={`chip ${tag === entry.id ? "active" : ""}`}
              onClick={() => setTag(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </div>
      </div>

      {!app.authenticated ? (
        <VerifyCard
          title="Verification required"
          detail="Verify once to load the fixes catalog for the games in your library."
          action={app.beginAuth}
        />
      ) : providerReady === false ? (
        <Empty
          icon={<Wrench />}
          title="Fixes are unavailable right now"
          detail="The catalog will appear here as soon as it is available. Everything else in Steamware keeps working."
        />
      ) : catalog.loading && games.length === 0 ? (
        <SkeletonGrid count={12} capsule />
      ) : catalog.error ? (
        <Empty icon={<Wrench />} title="Could not load fixes" detail={catalog.error.message} />
      ) : games.length === 0 ? (
        <Empty icon={<Wrench />} title="No fixes for that filter" detail="Try another tag or clear the search." />
      ) : (
        <div className="fix-grid">
          {games.map((game) => (
            <FixCard
              key={game.appid}
              game={game}
              onDownload={async (target: FixGame) => {
                const fix = target.fixes[0];
                if (!fix) return;
                await app.run(
                  () => api.startFixDownload(fix.id, null, target.name),
                  `Downloading fix for ${target.name}`,
                );
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/* ── Downloads ───────────────────────────────────────────────────────────────────────────────── */

const STATUS_WORD: Record<string, string> = {
  done: "Done",
  failed: "Failed",
  cancelled: "Cancelled",
  downloading: "Downloading",
  queued: "Queued",
};

export function DownloadsPage({ app, items }: { app: PageApi; items: DownloadItem[] }) {
  const active = items.filter((item) => item.status === "downloading" || item.status === "queued");
  const history = useAsync(() => api.downloadHistory(), [items.length]);
  const rows = history.data ?? [];

  return (
    <div className="page-inner dl-page">
      <div className="home-head">
        <h1>Downloads</h1>
        {active.length > 0 && <span className="pill">{active.length} active</span>}
      </div>

      {active.length === 0 ? (
        <div className="card dl-idle">Nothing downloading right now. Start downloading some shit!</div>
      ) : (
        <div className="card" style={{ overflow: "hidden" }}>
          {active.map((item) => (
            <DownloadRow
              key={item.id}
              item={item}
              onCancel={() => app.run(() => api.downloadCancel(item.id))}
              onReveal={() => app.run(() => api.revealPath(item.path))}
              onInstall={async () => {
                const done = await app.run(() => api.installLua(item.path));
                if (done) {
                  app.notify(
                    "ok",
                    "Installed into Steam",
                    done.live
                      ? `${done.lua} lua · ${done.manifests} manifest${done.manifests === 1 ? "" : "s"} — Steam picks them up straight away.`
                      : `${done.lua} lua · ${done.manifests} manifest${done.manifests === 1 ? "" : "s"} — restart Steam once so it reads them.`,
                  );
                }
              }}
            />
          ))}
        </div>
      )}

      <div className="dl-history">
        <div className="dl-history-head">
          <h2>History</h2>
          {rows.length > 0 && (
            <button
              className="btn small ghost"
              onClick={async () => {
                await app.run(() => api.downloadHistoryClear());
                history.reload();
              }}
            >
              Clear history
            </button>
          )}
        </div>

        {rows.length === 0 ? (
          <div className="card dl-idle small">
            Nothing here yet. Downloads you start from the Store, Library or Fixes page are kept here.
          </div>
        ) : (
          <div className="card dl-history-list">
            {rows.map((entry) => (
              <div className="dl-hist-row" key={entry.id}>
                <div className="grow" style={{ minWidth: 0 }}>
                  <div className="name" title={entry.title}>
                    {entry.title}
                  </div>
                  <div className="meta">
                    {entry.kind === "fix" ? "Fix" : "DepotBox"} · {STATUS_WORD[entry.status] ?? entry.status} ·{" "}
                    {formatBytes(entry.size)}
                  </div>
                </div>
                <span className="dl-when">
                  {new Date(entry.startedAt * 1000).toLocaleString(undefined, {
                    month: "2-digit",
                    day: "2-digit",
                    year: "numeric",
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </span>
                <button
                  className="icon-btn"
                  title="Remove from history"
                  aria-label="Remove from history"
                  onClick={async () => {
                    await app.run(() => api.downloadHistoryRemove(entry.id));
                    history.reload();
                  }}
                >
                  <X size={13} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

    </div>
  );
}

/* ── Settings ────────────────────────────────────────────────────────────────────────────────── */

/** A Settings line: what it does on the left, the control that changes it on the right. */
function SetRow({
  title,
  desc,
  children,
}: {
  title: ReactNode;
  desc?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="set-row">
      <div className="grow">
        <div className="set-title">{title}</div>
        {desc && <div className="set-desc">{desc}</div>}
      </div>
      {children && <div className="set-control">{children}</div>}
    </div>
  );
}

/** A Settings line whose control needs the whole row: a path, a key, a choice with a long label. */
function SetBlock({
  title,
  desc,
  children,
}: {
  title: ReactNode;
  desc?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="set-row column">
      <div className="grow">
        <div className="set-title">{title}</div>
      </div>
      <div className="set-control stretch">{children}</div>
      {desc && <div className="set-desc">{desc}</div>}
    </div>
  );
}

export function SettingsPage({
  app,
  boot,
  onSaved,
}: {
  app: PageApi;
  boot: Bootstrap;
  onSaved: (settings: Settings) => void;
}) {
  const [draft, setDraft] = useState<Settings>(boot.settings);
  const [saving, setSaving] = useState(false);
  const [steam, setSteam] = useState<SteamStatus>(boot.steam);
  const [showKey, setShowKey] = useState(false);
  const [checkingKey, setCheckingKey] = useState(false);
  // What Hubcap said about the key on this page. Held apart from the draft: it is the result of a check,
  // not a setting, and it has to be cleared when the key is edited.
  const [keyNote, setKeyNote] = useState<HubcapCheck | null>(null);
  // DepotBox is the built-in source; whether the server can serve it is server state, so it is read here
  // rather than configured. Hubcap is the opposite: the key is the user's own and lives on this PC.
  const sources = useAsync<ServerConfig>(() => api.serverConfig(), []);
  const depotboxReady = sources.data?.sources.find((source) => source.id === "depotbox")?.manifests === true;
  const hubcapReady = draft.hubcapKey.trim().length > 0;

  const comparable = (settings: Settings) => JSON.stringify(settings);
  const dirty = comparable(draft) !== comparable(boot.settings);

  // Appearance previews live: pick a colour and the whole window follows. Every change is written as it is
  // made, so leaving the page keeps it rather than putting the old pair back.
  useEffect(() => {
    applyTheme(draft.accent, draft.mode);
  }, [draft.accent, draft.mode]);

  // "Start with Windows" is a registry value, not a profile field. Read back what Windows will actually do:
  // Task Manager can disable the entry without telling the app, and a switch that shows the stored wish
  // instead of the live state is the thing that makes a working option look broken.
  useEffect(() => {
    let live = true;
    void api
      .startupStatus()
      .then((registered) => {
        if (!live) return;
        setDraft((previous) =>
          previous.startWithWindows === registered ? previous : { ...previous, startWithWindows: registered },
        );
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  // Every change is written on a short debounce rather than behind a Save button. The delay keeps a typed
  // path or key from writing the profile on every letter, and a change made just before leaving the page is
  // flushed on unmount instead of being lost with the component.
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const touched = useRef(false);
  const onSavedRef = useRef(onSaved);
  onSavedRef.current = onSaved;
  const [saveError, setSaveError] = useState(false);

  useEffect(() => {
    if (!touched.current || !dirty) return;
    const timer = window.setTimeout(() => {
      setSaving(true);
      setSaveError(false);
      void api
        .saveSettings({ ...draftRef.current })
        .then(async (saved) => {
          onSavedRef.current(saved);
          setSteam(await api.steamRefresh());
        })
        .catch((error) => {
          setSaveError(true);
          app.notify("err", toFailure(error).message);
        })
        .finally(() => setSaving(false));
    }, 400);
    return () => window.clearTimeout(timer);
  }, [draft, dirty]);

  useEffect(() => {
    return () => {
      if (!touched.current || !dirtyRef.current) return;
      void api
        .saveSettings({ ...draftRef.current })
        .then((saved) => onSavedRef.current(saved))
        .catch(() => {});
    };
  }, []);

  function patch(change: Partial<Settings>) {
    touched.current = true;
    setDraft((previous) => ({ ...previous, ...change }));
  }

  /**
   * The Hubcap key is the one field worth checking before it is kept: a wrong key fails every fetch that
   * relies on it, later, with a download error that says nothing about the key.
   *
   * The check goes to Hubcap directly, with the user's own key — the same call a Hubcap download makes. It
   * never touches the Steamware server. The key is saved like every other field; the check is what says
   * whether Hubcap accepts it.
   */
  async function validateKey() {
    const key = draft.hubcapKey.trim();
    setCheckingKey(true);
    try {
      const check = await api.hubcapCheck(key);
      setKeyNote(check);
      if (!check.ok) return;
      const saved = await api.saveSettings({ ...draft, hubcapKey: key });
      setDraft(saved);
      onSaved(saved);
      app.notify("ok", "Hubcap key saved", check.message);
    } catch (error) {
      setKeyNote({ ok: false, message: toFailure(error).message });
    } finally {
      setCheckingKey(false);
    }
  }

  return (
    <div className="page-inner settings-page">
      <PageHead
        title="Settings"
        subtitle="How Steamware behaves, where it looks for Steam, and where a fetch gets its files."
        actions={
          <span className={`settings-status ${saveError ? "failed" : ""}`} aria-live="polite">
            {saveError ? "Not saved" : saving || (touched.current && dirty) ? "Saving…" : "Saved"}
          </span>
        }
      />

      <div className="settings-sheet">
        {/* ── Install ────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Install</h2><p>Source choice and update pinning.</p></header>
          <div className="settings-section-body">
            <SetRow
              title="Keep games updated"
              desc="Installs without pinning a version, so Steam can update a game later. Turn it off to lock a game to the version you downloaded."
            >
              <Toggle
                checked={draft.autoUpdateApps}
                onChange={(value) => patch({ autoUpdateApps: value })}
                label="Auto update apps"
              />
            </SetRow>
            <SetRow
              title="FastFetch"
              desc="Check the source before downloading, so a game the first one cannot serve comes from the other."
            >
              <Toggle
                checked={draft.fastFetch}
                onChange={(value) => patch({ fastFetch: value })}
                label="FastFetch"
              />
            </SetRow>
            <SetBlock
              title="Default source"
              desc="Tried first whenever a download starts. If it can't serve the game, the other source is still used."
            >
              <select
                className="set-select"
                value={draft.preferredSource}
                aria-label="Default source"
                onChange={(event) =>
                  patch({ preferredSource: event.target.value as Settings["preferredSource"] })
                }
              >
                <option value="auto">Automatic (recommended)</option>
                <option value="depotbox">DepotBox</option>
                <option value="hubcap" disabled={!hubcapReady}>
                  Hubcap{hubcapReady ? "" : " (add a key first)"}
                </option>
              </select>
            </SetBlock>
          </div>
        </section>

        {/* ── Steam ──────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Steam</h2><p>Where Steam lives and what it holds.</p></header>
          <div className="settings-section-body">
            <SetRow
              title={
                <>
                  <span className={`status-dot ${steam.running ? "on" : ""}`} style={{ marginRight: 7 }} />
                  {steam.installed ? (steam.running ? "Steam is running" : "Steam detected") : "Steam not found"}
                </>
              }
              desc={
                steam.installed
                  ? `${steam.libraryCount} ${steam.libraryCount === 1 ? "library" : "libraries"}, ${steam.ownedCount} ${steam.ownedCount === 1 ? "game" : "games"} owned, ${steam.gameCount} added`
                  : "Install Steam, or point at its folder below."
              }
            >
              <button className="btn small ghost" onClick={async () => setSteam(await api.steamRefresh())}>
                <RefreshCw size={12} />
                Rescan
              </button>
            </SetRow>
            <SetBlock
              title="Steam location"
              desc={
                draft.steamPath.trim()
                  ? "Set by you — auto-detection is skipped."
                  : "Auto-detected from the registry. Change it if Steam lives somewhere unusual."
              }
            >
              <input
                className="set-input wide"
                type="text"
                value={draft.steamPath}
                placeholder={steam.installPath ?? "Not detected yet"}
                onChange={(event) => patch({ steamPath: event.target.value })}
                spellCheck={false}
                aria-label="Steam install folder"
                title={draft.steamPath || steam.installPath || ""}
              />
              <button
                className="btn small ghost"
                onClick={async () => {
                  const folder = await api.pickFolder();
                  if (folder) patch({ steamPath: folder });
                }}
              >
                Change…
              </button>
              <button
                className="btn small ghost"
                disabled={!steam.installPath}
                onClick={() => steam.installPath && app.run(() => api.revealPath(steam.installPath as string))}
              >
                <FolderOpen size={12} />
                Open
              </button>
              {draft.steamPath.trim() && (
                <button
                  className="icon-btn"
                  title="Clear and detect automatically"
                  aria-label="Clear the custom folder"
                  onClick={() => patch({ steamPath: "" })}
                >
                  <X size={13} />
                </button>
              )}
            </SetBlock>
          </div>
        </section>

        {/* ── Startup ────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Startup</h2><p>Windows start, and closing the window.</p></header>
          <div className="settings-section-body">
            <SetRow title="Start with Windows" desc="Launch Steamware automatically when Windows starts.">
              <Toggle
                checked={draft.startWithWindows}
                onChange={(value) => patch({ startWithWindows: value })}
                label="Start with Windows"
              />
            </SetRow>
            <SetRow
              title="Minimize to system tray"
              desc="When you close the window, keep Steamware running in the system tray instead of quitting. Double-click the tray icon to restore it."
            >
              <Toggle
                checked={draft.minimizeToTray}
                onChange={(value) => patch({ minimizeToTray: value })}
                label="Minimize to the system tray"
              />
            </SetRow>
            <SetRow
              title="Discord rich presence"
              desc="Show the page you're on — or the game you're downloading — on your Discord profile. The card carries a Download button for the community."
            >
              <Toggle
                checked={draft.discordRpc}
                onChange={(value) => patch({ discordRpc: value })}
                label="Discord Rich Presence"
              />
            </SetRow>
          </div>
        </section>

        {/* ── API ────────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>API</h2><p>The optional second source.</p></header>
          <div className="settings-section-body">
            <SetRow
              title="DepotBox"
              desc="Built in. Steamware uses it for luas and manifests out of the box."
            >
              <span className={`pill ${depotboxReady ? "ok" : "warn"}`}>
                {depotboxReady ? "Ready" : "Unavailable"}
              </span>
            </SetRow>
            <SetBlock
              title="Hubcap API key"
              desc={
                <>
                  Optional. Paste your personal Hubcap API key to fetch from Hubcap. The key is kept on this PC
                  and sent only to Hubcap. Get one at{" "}
                  <button
                    className="linkish"
                    onClick={() => app.run(() => api.openExternal("https://hubcapmanifest.com"))}
                  >
                    hubcapmanifest.com
                  </button>
                  .
                </>
              }
            >
              <input
                className="set-input wide"
                type={showKey ? "text" : "password"}
                value={draft.hubcapKey}
                placeholder="Paste your API key (smm_…)"
                onChange={(event) => {
                  patch({ hubcapKey: event.target.value });
                  setKeyNote(null);
                }}
                spellCheck={false}
                autoComplete="off"
                aria-label="Hubcap API key"
              />
              <button
                className="icon-btn"
                title={showKey ? "Hide the key" : "Show the key"}
                aria-label={showKey ? "Hide the key" : "Show the key"}
                onClick={() => setShowKey((visible) => !visible)}
              >
                {showKey ? <EyeOff size={13} /> : <Eye size={13} />}
              </button>
              <button
                className="btn small primary"
                onClick={validateKey}
                disabled={checkingKey || !draft.hubcapKey.trim()}
              >
                {checkingKey ? <Loader2 size={12} className="spin" /> : <Check size={12} />}
                Validate key
              </button>
            </SetBlock>
            {keyNote && (
              <div className="set-row column">
                <div className={`set-desc ${keyNote.ok ? "note-ok" : "note-warn"}`}>{keyNote.message}</div>
              </div>
            )}
          </div>
        </section>

        {/* ── Steamware Appearance ───────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Appearance</h2><p>Theme and accent, for this window.</p></header>
          <div className="settings-section-body">
            <SetRow
              title="Theme"
              desc="Light or dark, applied the moment you pick it — this window only."
            >
              <div className="seg">
                <button className={draft.mode === "dark" ? "active" : ""} onClick={() => patch({ mode: "dark" })}>
                  Dark
                </button>
                <button className={draft.mode === "light" ? "active" : ""} onClick={() => patch({ mode: "light" })}>
                  Light
                </button>
              </div>
            </SetRow>
            <div className="set-row column">
              <div className="grow">
                <div className="set-title">Accent colour</div>
                <div className="set-desc">
                  Highlights, toggles and buttons follow this colour, and so does the panel Steamware adds
                  inside Steam.
                </div>
              </div>
              {/* Under the label rather than beside it: twenty-five swatches cannot share a row with a
                  sentence without squeezing it to one word per line. */}
              <div className="swatches">
                {PALETTES.map((palette) => (
                  <button
                    key={palette.id}
                    className={`swatch ${draft.accent.toLowerCase() === palette.hex ? "active" : ""}`}
                    style={{ background: palette.hex }}
                    title={palette.name}
                    aria-label={`${palette.name} accent`}
                    onClick={() => patch({ accent: palette.hex })}
                  />
                ))}
              </div>
            </div>
          </div>
        </section>


        {/* ── Account ────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Account</h2><p>The check that unlocks downloads.</p></header>
          <div className="settings-section-body">
            <SetRow
              title={app.authenticated ? "Verified" : "Not verified"}
              desc={
                app.authenticated
                  ? `Store search, manifests and fixes are unlocked. The check renews when this session ends${app.session ? ` in ${remaining(app.session.expiresAt - Math.floor(Date.now() / 1000))}` : ""}.`
                  : "A one-tap Cloudflare check unlocks the store, manifests and fixes."
              }
            >
              {app.authenticated ? (
                <button className="btn small ghost" onClick={() => app.run(() => api.authSignOut(), "Signed out")}>
                  Sign out
                </button>
              ) : (
                <button className="btn small primary" onClick={app.beginAuth}>
                  <ShieldCheck size={12} />
                  Verify
                </button>
              )}
            </SetRow>
          </div>
        </section>

        {/* ── Maintenance ────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>Maintenance</h2><p>History and this profile's files.</p></header>
          <div className="settings-section-body">
            <SetRow
              title="Download history"
              desc="The list on the Downloads page. Clearing it never touches the files."
            >
              <button
                className="btn small ghost"
                onClick={() =>
                  app.run(async () => {
                    await api.downloadHistoryClear();
                  }, "History cleared")
                }
              >
                <Trash2 size={12} />
                Clear
              </button>
            </SetRow>
            <SetRow
              title="Profile folder"
              desc="Settings, download history and the device id for this installation."
            >
              <button className="btn small ghost" onClick={() => app.run(() => api.revealPath(boot.dataDir))}>
                <FolderOpen size={12} />
                Open
              </button>
            </SetRow>
          </div>
        </section>

        {/* ── About ──────────────────────────────────────────────────────────────────────── */}
        <section className="settings-section">
          <header className="settings-section-head"><h2>About</h2><p>Version and the community.</p></header>
          <div className="settings-section-body">
            <SetRow
              title={`Steamware ${versionTag(app.version)}`}
              desc="Made by the SteamTools community. Help, updates and announcements live in the Discord."
            >
              <button
                className="btn small ghost"
                onClick={() => app.run(() => api.openExternal("https://discord.gg/steamtools"))}
              >
                <MessageCircle size={12} />
                discord.gg/steamtools
              </button>
            </SetRow>
          </div>
        </section>
      </div>
    </div>
  );
}

/* ── Onboarding ─────────────────────────────────────────────────────────────────────────────── */

/**
 * The first run, as two questions and nothing else: the recommended settings, then the plugin.
 *
 * The order is the point. The offer that changes Steam itself comes first, and its answer stays a choice
 * until "Next" is pressed, so reading the wizard installs nothing. Pressing Next fades the body out and
 * brings the next question in, and the last question's button reads "Done" instead, because that one closes
 * the sheet and does the work the two answers described.
 *
 * A server step still comes before them, but only for a source checkout built without an address: a
 * released copy has its address baked in, and letting a user type one is the risk this removes.
 */
type OnboardStep = "server" | "recommended" | "plugin";

export function Onboarding({
  boot,
  app,
  onFinished,
}: {
  boot: Bootstrap;
  app: PageApi;
  onFinished: (settings: Settings) => void;
}) {
  const needsServer = !boot.defaultApiBase && !boot.settings.apiBase;
  const flow: OnboardStep[] = needsServer ? ["server", "recommended", "plugin"] : ["recommended", "plugin"];
  const [step, setStep] = useState<OnboardStep>(needsServer ? "server" : "recommended");
  const [apiBase, setApiBase] = useState(boot.settings.apiBase || boot.defaultApiBase);
  const [saving, setSaving] = useState(false);
  // Both first-run offers default to yes, exactly as the WPF wizard did: the recommended path is the one
  // almost everyone takes, so making it the default costs nobody a click.
  const [applyRecommended, setApplyRecommended] = useState(true);
  const [installPlugin, setInstallPlugin] = useState(true);

  // An install that already has the unlocker and the plugin — an upgrade from the WPF build, or a second run
  // after the profile was cleared — is not asked to install them again. The questions stay, preselected No,
  // so nothing is reinstalled behind the user's back.
  useEffect(() => {
    let alive = true;
    Promise.all([api.pluginState(), api.modesState()])
      .then(([plugin, modes]) => {
        if (!alive) return;
        if (plugin.installed) setInstallPlugin(false);
        if (modes.cards.some((card) => card.active)) setApplyRecommended(false);
      })
      .catch(() => {
        // A wizard that cannot read this state just keeps both answers at their default.
      });
    return () => {
      alive = false;
    };
  }, []);

  // The body leaves before the next question arrives, rather than being swapped in one frame: these are two
  // separate questions and the transition is what says so. A machine that asked for reduced motion gets the
  // change at once — the fade is decoration, and the answer is not.
  const [leaving, setLeaving] = useState(false);
  const pending = useRef<number | null>(null);
  const settleFor = () =>
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 180;

  useEffect(
    () => () => {
      if (pending.current !== null) window.clearTimeout(pending.current);
    },
    [],
  );

  // Addressed by name, not by number: the server question drops out of the flow the moment an address is
  // saved, and an index would then point at a different question than the one that was on screen.
  function go(next: OnboardStep) {
    if (next === step || leaving) return;
    const wait = settleFor();
    if (wait === 0) {
      setStep(next);
      return;
    }
    setLeaving(true);
    pending.current = window.setTimeout(() => {
      setStep(next);
      setLeaving(false);
      pending.current = null;
    }, wait);
  }

  async function saveServer() {
    setSaving(true);
    try {
      const saved = await api.saveSettings({
        ...boot.settings,
        apiBase: apiBase.trim(),
      });
      onFinished(saved);
      go("recommended");
    } catch (error) {
      app.notify("err", toFailure(error).message);
    } finally {
      setSaving(false);
    }
  }

  /**
   * Close the wizard, then do the work it offered.
   *
   * The save comes first and the installs follow it, unanswered: the sheet disappears the moment the user
   * presses Done, and a slow download cannot leave the welcome screen stuck on top of the app. Each step
   * reports through the same toasts the rest of the app uses, so a failure is named rather than swallowed.
   */
  async function finish() {
    try {
      // The address written back is never a typed one: the profile's own, or the build's baked-in value for
      // a profile that somehow has neither.
      const saved = await api.saveSettings({
        ...boot.settings,
        apiBase: boot.settings.apiBase || boot.defaultApiBase,
        completedOnboarding: true,
      });
      onFinished(saved);
    } catch (error) {
      app.notify("err", toFailure(error).message);
      return;
    }

    if (!applyRecommended && !installPlugin) return;
    app.notify("ok", "Applying your choices", "Steam is updated in the background — you can keep using the app.");

    if (applyRecommended) {
      try {
        await api.modesInstall("bst");
        app.notify("ok", "Recommended settings applied", "BetterSteamTools is installed. Switch it any time on the Mode page.");
      } catch (error) {
        app.notify("err", toFailure(error).message, "The unlocker was not installed. Mode → Switch to this retries it.");
      }
    }

    if (installPlugin) {
      try {
        await api.pluginInstall();
        app.notify("ok", "Plugin installed", "Steam picks it up on its next start.");
      } catch (error) {
        app.notify("err", toFailure(error).message, "The plugin was not installed. Plugin → Install retries it.");
      }
    }
  }

  return (
    <div className="onboarding">
      <div className="sheet">
        <div className="brand">
          <BrandMark size={26} />
          <span style={{ fontWeight: 600 }}>Steamware</span>
        </div>

        <div className="steps">
          {flow.map((_, index) => (
            <div key={index} className={`step ${flow.indexOf(step) >= index ? "done" : ""}`} />
          ))}
        </div>

        <div className={`onboard-body ${leaving ? "leaving" : ""}`}>
          {step === "server" && (
            <>
              <h1>Point Steamware at your server</h1>
              <p className="lead">
                Steamware talks to one host: the Cloudflare Worker you deploy. It holds the provider keys, checks
                that you're human, and proxies manifests and fixes. Nothing secret ships in this app.
              </p>
              <div className="field">
                <label htmlFor="onboard-url">Worker address</label>
                <input
                  id="onboard-url"
                  type="url"
                  autoFocus
                  value={apiBase}
                  placeholder="https://steamware-api.your-name.workers.dev"
                  onChange={(event) => setApiBase(event.target.value)}
                  spellCheck={false}
                />
                <div className="hint">
                  You get this the moment you run <span className="kbd">wrangler deploy</span>. The setup guide ships
                  in <span className="kbd">backend/README.md</span>.
                </div>
              </div>
              <div className="actions">
                <div className="grow" />
                <button className="btn primary" onClick={saveServer} disabled={!apiBase.trim() || saving}>
                  {saving ? <Loader2 size={13} className="spin" /> : <ArrowRight size={13} />}
                  Continue
                </button>
              </div>
            </>
          )}

          {step === "recommended" && (
            <>
              <h1>Apply recommended settings?</h1>
              <p className="lead">
                Sets Steamware up the way it is meant to run: the recommended unlocker goes in, so games you add
                are ready to play without another trip to the Mode page.
              </p>
              <ul className="bullets">
                <li>
                  <Wrench />
                  Installs BetterSteamTools, the unlocker marked Recommended on the Mode page.
                </li>
                <li>
                  <Layers />
                  Leaves your depots, pins and saved builds alone — it only adds the unlocker.
                </li>
              </ul>
              <div className="choices">
                <button
                  className={`btn ${applyRecommended ? "primary" : "ghost"}`}
                  onClick={() => setApplyRecommended(true)}
                  aria-pressed={applyRecommended}
                >
                  <Check size={13} />
                  Yes, use the recommended settings
                </button>
                <button
                  className={`btn ${applyRecommended ? "ghost" : "primary"}`}
                  onClick={() => setApplyRecommended(false)}
                  aria-pressed={!applyRecommended}
                >
                  No, I'll choose myself
                </button>
              </div>
              <div className="actions">
                {needsServer && (
                  <button className="btn ghost" onClick={() => go("server")}>
                    Back
                  </button>
                )}
                <div className="grow" />
                <button className="btn primary" onClick={() => go("plugin")}>
                  Next
                  <ArrowRight size={13} />
                </button>
              </div>
            </>
          )}

          {step === "plugin" && (
            <>
              <h1>Install the Steamware plugin?</h1>
              <p className="lead">
                Adds the great Steamware plugin to Steam's store pages, so the game you are looking at is one click
                from being added. It writes inside your Steam folder and can be removed again from the Plugin page.
              </p>
              <ul className="bullets">
                <li>
                  <Sparkles />
                  A button on every store page, next to Steam's own.
                </li>
                <li>
                  <ShieldCheck />
                  Downloaded and verified before it lands; uninstalling removes every file.
                </li>
              </ul>
              <div className="choices">
                <button
                  className={`btn ${installPlugin ? "primary" : "ghost"}`}
                  onClick={() => setInstallPlugin(true)}
                  aria-pressed={installPlugin}
                >
                  <Check size={13} />
                  Yes, install the plugin
                </button>
                <button
                  className={`btn ${installPlugin ? "ghost" : "primary"}`}
                  onClick={() => setInstallPlugin(false)}
                  aria-pressed={!installPlugin}
                >
                  No, not right now
                </button>
              </div>
              <div className="actions">
                <button className="btn ghost" onClick={() => go("recommended")}>
                  Back
                </button>
                <div className="grow" />
                <button className="btn primary" onClick={finish}>
                  <Check size={13} />
                  Done
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

export { Empty, Search, Trash2, steamHeader };
