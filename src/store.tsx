import { useEffect, useMemo, useState } from "react";

import { ArrowLeft, ChevronRight, Download, Loader2, Store, X } from "lucide-react";
import {
  api,
  formatBytes,
  formatCount,
  refreshGameStats,
  useAsync,
  coverCandidates,
  heroCandidates,
  useCoverChain,
  useEvenColumns,
  useStatsAutoRefresh,
  type DepotInfo,
  type LuaState,
  type ServerConfig,
  type StoreApp,
  type StoreItem,
} from "./lib";
import {
  ArtTile,
  Empty,
  FileDropZone,
  PageHead,
  SearchField,
  SectionHead,
} from "./components";
import type { PageApi } from "./pages";

const GENRES = [
  "Action",
  "Adventure",
  "RPG",
  "Strategy",
  "Simulation",
  "Racing",
  "Sports",
  "Casual",
  "Indie",
  "Massively Multiplayer",
  "Free to Play",
];

function AutocompleteRow({ item, onClick }: { item: StoreItem; onClick: () => void }) {
  const cover = useCoverChain(coverCandidates(item.appid, item.cover));
  return (
    <button className="ac-row" onClick={onClick} type="button">
      <span className="ac-art">{cover ? <img src={cover} alt="" loading="lazy" /> : null}</span>
      <span className="ac-name">{item.name}</span>
      <span className="ac-id">{item.appid}</span>
      <ChevronRight size={14} />
    </button>
  );
}

/** The catalog line under the title, straight from the provider through the Worker. */
function CatalogSummary() {
  const stats = useAsync(() => api.catalogStats(), []);
  if (!stats.data) return null;
  return (
    <div className="store-summary">
      {formatCount(stats.data.games)} games in the Steamware database
    </div>
  );
}

function DetailPanel({
  appId,
  app,
  providerReady,
  onClose,
}: {
  appId: string;
  app: PageApi;
  providerReady: boolean;
  onClose: () => void;
}) {
  const detail = useAsync<StoreApp | null>(() => api.storeApp(appId), [appId]);
  // Depot detail is public Steam data too, so the panel fills in before anyone verifies — which is what
  // it did in the build this replaces.
  const info = useAsync<DepotInfo | null>(() => api.steamApp(appId), [appId]);
  // The game's own lua is the only honest answer to "which depots do I already have?" — it is what Steam
  // actually loads, not what the provider says is available.
  const lua = useAsync<LuaState | null>(() => api.buildsState(appId), [appId]);

  const enabled = useMemo(() => {
    const set = new Set<string>();
    for (const row of lua.data?.rows ?? []) if (row.enabled) set.add(row.id);
    return set;
  }, [lua.data]);

  const depots = (info.data?.depots ?? []).filter((depot) => !depot.shared);

  // What a fetch is actually made of: a size to plan disk around, how many of those depots are DLC, and — once
  // a lua exists — how many of them it already covers. The table this replaces listed every depot as "no key",
  // which was never about the depot: nothing had been fetched yet, so nothing was in the lua, and a row said the
  // same alarming thing whether or not that fetch would work.
  const sizedBytes = depots.reduce((sum, depot) => sum + (depot.size || 0), 0);
  const allSized = depots.every((depot) => (depot.size || 0) > 0);
  const dlcDepots = depots.filter((depot) => depot.dlcAppId).length;
  const covered = depots.filter((depot) => enabled.has(depot.id)).length;
  const hasLua = (lua.data?.rows?.length ?? 0) > 0;

  const data = detail.data;
  const genres = data?.genres?.slice(0, 3).join(" · ") ?? "";
  // The big art, not the 460×215 tile: this one is painted at full window width, where a header stretch
  // reads as a blurry stripe.
  const cover = useCoverChain(heroCandidates(appId, data?.headerImage));

  return (
    <section className="store-detail">
      <div className={`detail-hero ${cover ? "" : "empty"}`}>
        {cover ? (
          <img src={cover} alt="" decoding="async" />
        ) : (
          <div className="detail-hero-empty">
            <span>{data?.name ?? `App ${appId}`}</span>
          </div>
        )}
        <div className="detail-hero-shade" />
      </div>

      <div className="detail-grid">
        <div className="detail-main">
          <header className="detail-title">
            <div className="grow">
              <h2>{data?.name ?? `App ${appId}`}</h2>
              <div className="detail-meta">
                <span className="meta-chip mono">{appId}</span>
                {genres && <span className="meta-chip">{genres}</span>}
                {data?.releaseDate && <span className="meta-chip quiet">Released {data.releaseDate}</span>}
              </div>
            </div>
            <button className="icon-btn" onClick={onClose} title="Close" aria-label="Close">
              <X size={14} />
            </button>
          </header>

          {data?.shortDescription && <p className="detail-desc">{data.shortDescription}</p>}

          <div className="detail-cta">
            <button
              className="btn primary detail-fetch"
              disabled={!providerReady || lua.loading}
              title={
                providerReady
                  ? "Download this game's lua and manifests into Steam"
                  : "Fetching is unavailable right now — try again in a little while."
              }
              onClick={async () => {
                await app.run(
                  () => api.startManifestDownload(appId, data?.name ?? `App ${appId}`),
                  "Fetching — it installs into Steam when it lands",
                );
              }}
            >
              {lua.loading ? <Loader2 size={14} className="spin" /> : <Download size={14} />}
              Fetch &amp; install
            </button>
            <span className="detail-note">
              {providerReady
                ? "Pulls the lua and its manifests into Steam. It appears in your library as it lands."
                : "Fetching is temporarily unavailable. The store details below still work."}
            </span>
          </div>

          {depots.length > 0 && (
            <section className="detail-block">
              <div className="detail-block-head">
                <span className="detail-block-title">Contents</span>
                <span className="section-note">Steam's own depot data</span>
              </div>

              <div className="detail-facts">
                <div className="detail-fact">
                  <span className="fact-value">{formatCount(depots.length)}</span>
                  <span className="fact-label">depots</span>
                </div>
                {sizedBytes > 0 && (
                  <div className="detail-fact">
                    <span className="fact-value">
                      {allSized ? "" : "≈ "}
                      {formatBytes(sizedBytes)}
                    </span>
                    <span className="fact-label">install size</span>
                  </div>
                )}
                {dlcDepots > 0 && (
                  <div className="detail-fact">
                    <span className="fact-value">{formatCount(dlcDepots)}</span>
                    <span className="fact-label">from DLC</span>
                  </div>
                )}
                {hasLua && (
                  <div className="detail-fact">
                    <span className="fact-value">
                      {formatCount(covered)}/{formatCount(depots.length)}
                    </span>
                    <span className="fact-label">in your lua</span>
                  </div>
                )}
              </div>

              <p className="detail-note">
                {hasLua
                  ? "Depot-by-depot detail, manifests and pins are on the Builds page."
                  : "A fetch pulls whatever the source can key, so it can cover fewer depots than this. Depot detail appears on the Builds page afterwards."}
              </p>
            </section>
          )}
        </div>

        <aside className="detail-side">
          <div className="side-card">
            <span className="side-title">Manifest sources</span>
            <div className="source-row">
              <span className="name">DepotBox</span>
              <span className={`pill ${providerReady ? "ok" : "warn"}`}>
                {providerReady ? "Ready" : "Not ready"}
              </span>
            </div>
            <div className="source-row">
              <span className="name">Hubcap</span>
              <span className="pill">Not configured</span>
            </div>
          </div>

          <div className="side-card">
            <span className="side-title">Installs into Steam</span>
            <div className="side-path">
              <span className="kbd">steam\config\stplug-in</span> the game's lua
            </div>
            <div className="side-path">
              <span className="kbd">steam\depotcache</span> its manifests
            </div>
          </div>
        </aside>
      </div>
    </section>
  );
}

/**
 * What has been fetched for one chip.
 *
 * One entry per genre (and one for "All") rather than a single list, because a chip you come back to has to
 * repaint from ITS OWN cards: a shared list is how a genre grid ended up showing the previous genre's games,
 * with "Show more" then appending one genre's page onto the other's.
 */
type ChipPage = {
  items: StoreItem[];
  total: number;
  nextStart: number;
  exhausted: boolean;
  fetched: boolean;
  loading: boolean;
};

const emptyChip: ChipPage = { items: [], total: 0, nextStart: 0, exhausted: false, fetched: false, loading: false };

/** Steam caps a deep page; past this a request is wasted. */
const MAX_START = 2000;
/** Requested page size. Steam answers with at least 25 whatever is asked. */
const PAGE_SIZE = 24;

/// The narrowest a card may be drawn. It is set by what has to fit under the art, not by the art itself: at
/// 208 the two figures beneath every card get about seventy pixels each, which is enough for "1,036,384" and
/// for "9.95 M reviews" without either being cut off. A wider window adds columns rather than shrinking cards
/// past this, so the row holds two cards in a narrow window and eight on a very wide one.
/// The width a card is asked to be. A wider window adds columns until `GRID_MAX_COLUMNS` rather than
/// stretching the same games across the page, so the shelf reads the same at every size of window.
const GRID_TARGET_CARD = 200;
/// A 4K window still fills its row; the ceiling only stops cards becoming thumbnails.
const GRID_MAX_COLUMNS = 20;
const GRID_GAP = 14;

export function StorePage({ app }: { app: PageApi }) {
  const { ref: gridRef, cols } = useEvenColumns<HTMLDivElement>(GRID_TARGET_CARD, GRID_MAX_COLUMNS, GRID_GAP);
  // The review score and player count move while the page is open, and a return to the window is the moment
  // somebody is looking at them again.
  useStatsAutoRefresh();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [genre, setGenre] = useState<string | null>(null);
  const [pages, setPages] = useState<Record<string, ChipPage>>({});
  const [selected, setSelected] = useState<string | null>(null);
  const chipKey = genre ?? "";
  const page = pages[chipKey] ?? emptyChip;

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 300);
    return () => clearTimeout(timer);
  }, [query]);

  const config = useAsync<ServerConfig>(() => api.serverConfig(), []);
  const providerReady = config.data?.sources.find((source) => source.id === "depotbox")?.manifests === true;

  const search = useAsync(
    () => (debounced ? api.storeSearch(debounced) : Promise.resolve({ items: [] })),
    [debounced],
    { skip: !debounced },
  );

  const patch = (key: string, update: (previous: ChipPage) => ChipPage) => {
    setPages((current) => ({ ...current, [key]: update(current[key] ?? emptyChip) }));
  };

  const load = async (key: string, startAt: number, append: boolean) => {
    patch(key, (previous) => ({ ...previous, loading: true }));
    const fetched = await app.run(() => api.storeGenre(key, startAt));
    patch(key, (previous) => {
      if (!fetched) return { ...previous, loading: false };
      // Merge on the appid, keeping the first copy: Steam's own ordering does repeat entries, and a
      // duplicate card is the one thing every user notices.
      const seen = new Set(append ? previous.items.map((item) => String(item.appid)) : []);
      const items = append ? [...previous.items] : [];
      for (const item of fetched.items) {
        const id = String(item.appid);
        if (seen.has(id)) continue;
        seen.add(id);
        items.push(item);
      }
      // The cursor advances by what actually came back, not by what was asked for. Steam serves a
      // minimum of 25 however small the request, so stepping by the requested 24 lands back inside the
      // window just served and "Show more" would fetch the same cards again.
      const short = fetched.items.length === 0;
      const nextStart = startAt + fetched.items.length;
      return {
        items,
        total: fetched.total,
        nextStart,
        // A page shorter than asked for is the end of the list. `total` is only trusted when Steam sent
        // one — the plain `items` shape carries no count, and a count of the first page would end the
        // list after 25 games.
        exhausted:
          short || fetched.items.length < PAGE_SIZE || (fetched.total > 0 && nextStart >= fetched.total) || nextStart >= MAX_START,
        fetched: true,
        loading: false,
      };
    });
  };

  /**
   * The cards that are drawn: only whole rows.
   *
   * Steam answers a 24-card request with at least 25, so at eight columns the twenty-fifth game paints a
   * fourth row holding one card — the stray the shelf must not have. Everything up to the last complete row
   * is shown and the remainder waits: as soon as enough have arrived for another full row they all appear, so
   * nothing is lost, it is only held back.
   */
  const wholeRows = useMemo(() => {
    const items = page.items;
    if (items.length <= cols || cols <= 0) return items;
    return items.slice(0, Math.floor(items.length / cols) * cols);
  }, [page.items, cols]);

  // First visit to a chip (including "All", which is the same list with no genre) fetches its first page;
  // coming back to one already fetched repaints from memory with no request at all.
  useEffect(() => {
    const current = pages[chipKey];
    if (current?.fetched || current?.loading) return;
    void load(chipKey, 0, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chipKey]);

  async function loadMore() {
    if (page.loading || page.exhausted) return;
    await load(chipKey, page.nextStart, true);
  }

  const results = debounced ? search.data?.items ?? [] : [];
  const numeric = /^\d{1,10}$/.test(debounced);

  function open(appId: string | number) {
    setSelected(String(appId));
    setQuery("");
    setDebounced("");
  }

  // Coming back from a game's own page is the other half of "refreshed whenever I look at it again": the
  // grid is repainted with numbers read a moment ago instead of the ones it was left holding.
  useEffect(() => {
    if (!selected) refreshGameStats();
  }, [selected]);

  // A game opens as its own page, the way the WPF build did — the grid is replaced, not pushed down. Coming
  // back returns to the SAME chip and scroll position because the page state above is never unmounted.
  if (selected) {
    return (
      <div className="page-inner store-detail-page">
        <div className="detail-bar">
          <button className="btn ghost back" onClick={() => setSelected(null)}>
            <ArrowLeft size={14} />
            Back to store
          </button>
          <span className="section-note">
            {genre ? `${genre} · top sellers` : "All games · top sellers"}
          </span>
        </div>
        <DetailPanel appId={selected} app={app} providerReady={providerReady} onClose={() => setSelected(null)} />
      </div>
    );
  }

  return (
    <div className="page-inner">
      <PageHead
        title="Store"
        subtitle="Enter an App ID, paste a Steam link, or search by game name"
      />
      <CatalogSummary />

      <div className="store-search">
        {query.trim().length > 0 && (
          <button
            className="btn ghost back"
            onClick={() => {
              setQuery("");
              setDebounced("");
            }}
            title="Clear the search"
            aria-label="Clear the search"
          >
            <ArrowLeft size={14} />
          </button>
        )}
        <SearchField value={query} onChange={setQuery} placeholder="Enter App ID or search games…" />
      </div>

      {debounced && (
        <div className="ac-panel">
          {numeric && (
            <button className="ac-row" onClick={() => open(debounced)} type="button">
              <span className="ac-art appid">
                <span>{debounced}</span>
              </span>
              <span className="ac-name">Open App {debounced}</span>
              <ChevronRight size={14} />
            </button>
          )}
          {search.loading && (
            <div className="ac-empty">
              <span className="spinner" />
              Searching…
            </div>
          )}
          {!search.loading && results.length === 0 && !numeric && (
            <div className="ac-empty">Nothing matched. Try another name, or paste the App ID.</div>
          )}
          {results.map((item) => (
            <AutocompleteRow key={item.appid} item={item} onClick={() => open(item.appid)} />
          ))}
        </div>
      )}

      <div className="chips">
        <button className={`chip ${genre === null ? "active" : ""}`} onClick={() => setGenre(null)}>
          All
        </button>
        {GENRES.map((entry) => (
          <button
            key={entry}
            className={`chip ${genre === entry ? "active" : ""}`}
            onClick={() => setGenre(genre === entry ? null : entry)}
          >
            {entry}
          </button>
        ))}
      </div>

      <section className="store-section">
          <SectionHead title={genre ? `${genre} · top sellers` : "All games · top sellers"}>
            {page.loading && <span className="spinner" />}
            {page.total > 0 && <span className="section-note">{formatCount(page.total)} available</span>}
          </SectionHead>
          {page.loading && page.items.length === 0 ? (
            <div className="store-grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}>
              {Array.from({ length: cols * 3 }).map((_, index) => (
                <div className="tile" key={index}>
                  <div className="skeleton" style={{ aspectRatio: "460/215" }} />
                  <div className="skeleton" style={{ height: 10, width: "70%" }} />
                  <div className="skeleton" style={{ height: 34 }} />
                </div>
              ))}
            </div>
          ) : page.items.length === 0 ? (
            <Empty
              icon={<Store />}
              title="The store is quiet"
              detail="Try another genre, or search a name above."
            />
          ) : (
            <>
              <div
                className="store-grid"
                ref={gridRef}
                style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
              >
                {wholeRows.map((item) => (
                  <ArtTile
                    key={`${chipKey}-${item.appid}`}
                    appId={item.appid}
                    name={item.name}
                    cover={item.cover}
                    onClick={() => open(item.appid)}
                  />
                ))}
              </div>
              {/* Under the shelf, centred: it is the end of the list the reader has just walked down, and
                  in the header it competed with the section title for attention. */}
              {!page.exhausted && (
                <div className="store-more">
                  <button className="btn ghost" onClick={() => void loadMore()} disabled={page.loading}>
                    {page.loading ? "Loading…" : "Show more"}
                  </button>
                </div>
              )}
            </>
          )}
      </section>

      <div className="already">
        <div className="already-title">Already have the files?</div>
        <FileDropZone
          onPicked={async () => {
            const path = await app.run(() => api.pickArchive());
            if (!path) return;
            const done = await app.run(() => api.installLua(path));
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
          title="Drop .lua, .manifest or .zip files here"
          subtitle="— or drag a SteamDB / Steam store link to add that game."
        />
      </div>

    </div>
  );
}
