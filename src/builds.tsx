import { useEffect, useMemo, useState } from "react";
import {
  Bookmark,
  Check,
  Download,
  GitBranch,
  Library,
  Pencil,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import {
  api,
  formatBytes,
  useAsync,
  useCover,
  steamHeader,
  toFailure,
  type BuildGame,
  type DepotInfo,
  type LuaRow,
  type LuaState,
} from "./lib";
import { Empty, PageHead, SearchField, SectionHead } from "./components";
import type { PageApi } from "./pages";

const PAGE_SIZE = 30;

/** The depot facts Steam publishes for a game, keyed by id, for sizes and "behind the latest build". */
function depotIndex(info: DepotInfo | null) {
  const map = new Map<string, DepotInfo["depots"][number]>();
  for (const depot of info?.depots ?? []) map.set(depot.id, depot);
  return map;
}

function GameRow({
  game,
  active,
  onClick,
}: {
  game: BuildGame;
  active: boolean;
  onClick: () => void;
}) {
  const cover = useCover(steamHeader(game.appId));
  const badge = game.variantCount > 0 ? `${game.variantCount} builds` : game.hasLua ? "Default" : null;

  return (
    <button className={`game-row ${active ? "active" : ""}`} onClick={onClick} type="button">
      <span className="thumb">{cover ? <img src={cover} alt="" loading="lazy" /> : null}</span>
      <span className="grow">
        <span className="title">{game.name}</span>
        {badge && <span className="meta">{badge}</span>}
      </span>
    </button>
  );
}

function Toggle({
  checked,
  disabled,
  onChange,
  label,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`switch ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

function DepotRowView({
  row,
  depot,
  onToggle,
}: {
  row: LuaRow;
  depot: DepotInfo["depots"][number] | undefined;
  onToggle: (field: "enabled" | "locked", value: boolean) => void;
}) {
  const title =
    row.comment ??
    (row.kind === "base" ? "Base app" : row.kind === "dlc" ? `DLC ${row.id}` : `Depot ${row.id}`);
  const meta = [
    row.id,
    row.size ? formatBytes(row.size) : depot?.size ? formatBytes(depot.size) : null,
    depot?.os ?? null,
    depot?.language ?? null,
  ]
    .filter(Boolean)
    .join(" · ");
  const outdated = Boolean(row.locked && row.manifest && depot?.manifestId && depot.manifestId !== row.manifest);

  return (
    <div className="build-row">
      <Toggle
        checked={row.enabled}
        onChange={(value) => onToggle("enabled", value)}
        label={`${row.enabled ? "Disable" : "Enable"} ${title}`}
      />
      <div className="grow">
        <div className="title">
          {title}
          {row.kind === "dlc" && <span className="tag-pill">DLC</span>}
          {row.kind === "depot" && depot?.shared && <span className="tag-pill gold">SHARED</span>}
        </div>
        <div className="meta">{meta}</div>
        {row.manifest && (
          <div className={`manifest ${row.locked ? "" : "dim"}`}>manifest {row.manifest}</div>
        )}
        {outdated && <div className="manifest warn">latest is {depot?.manifestId}</div>}
      </div>
      {row.canLock && (
        <label className="lock">
          <input
            type="checkbox"
            checked={row.locked}
            onChange={(event) => onToggle("locked", event.target.checked)}
          />
          Lock
        </label>
      )}
    </div>
  );
}

export function BuildsPage({
  app,
  onOpenLibrary,
  focus,
}: {
  app: PageApi;
  onOpenLibrary?: () => void;
  focus?: { appId: string; at: number } | null;
}) {
  const games = useAsync(() => api.buildsList(), []);
  const config = useAsync(() => api.serverConfig(), []);
  const canDownload = config.data?.sources.find((source) => source.id === "depotbox")?.manifests;
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [lua, setLua] = useState<LuaState | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [renaming, setRenaming] = useState(false);
  const [renameText, setRenameText] = useState("");
  const [naming, setNaming] = useState(false);
  const [newName, setNewName] = useState("");
  const [rowQuery, setRowQuery] = useState("");
  const [perPage, setPerPage] = useState(PAGE_SIZE);

  const info = useAsync<DepotInfo | null>(
    () => (selected ? api.steamApp(selected) : Promise.resolve(null)),
    [selected],
    { skip: !selected },
  );
  const depots = useMemo(() => depotIndex(info.data), [info.data]);

  const filtered = (games.data ?? []).filter((game) =>
    game.name.toLowerCase().includes(query.trim().toLowerCase()),
  );
  // 0 means "All": the picker then lists the whole library on a single page.
  const size = perPage === 0 ? Math.max(filtered.length, 1) : perPage;
  const pages = Math.max(1, Math.ceil(filtered.length / size));
  const current = Math.min(page, pages - 1);
  const visible = filtered.slice(current * size, current * size + size);

  useEffect(() => {
    setPage(0);
  }, [query]);

  // A library game's "Manage build" opens this page already on that game, the way the Windows build's
  // flyout did. `at` makes asking again for the same game a fresh request.
  useEffect(() => {
    if (!focus?.appId) return;
    setQuery("");
    setSelected(focus.appId);
  }, [focus?.at, focus?.appId]);

  useEffect(() => {
    if (!selected) {
      setLua(null);
      return;
    }
    let alive = true;
    setLoading(true);
    setEditing(false);
    api
      .buildsState(selected)
      .then((state) => alive && setLua(state))
      .catch((error) => alive && app.notify("err", toFailure(error).message))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  async function mutate(task: () => Promise<LuaState>, success?: string) {
    setBusy(true);
    try {
      const state = await task();
      setLua(state);
      if (success) app.notify("ok", success);
      games.reload();
      return state;
    } catch (error) {
      app.notify("err", toFailure(error).message);
      return null;
    } finally {
      setBusy(false);
    }
  }

  const rows = (lua?.rows ?? []).filter((row) => {
    const needle = rowQuery.trim().toLowerCase();
    if (!needle) return true;
    return (
      row.id.includes(needle) ||
      (row.comment ?? "").toLowerCase().includes(needle) ||
      (depots.get(row.id)?.dlcAppId ?? "").includes(needle)
    );
  });
  const activeRows = rows.filter((row) => row.enabled);
  const disabledRows = rows.filter((row) => !row.enabled);

  const activeVariant = lua?.variants.find((variant) => variant.id === lua.activeVariant) ?? null;

  return (
    <div className="page-inner">
      <PageHead
        title="Builds"
        subtitle="Keep multiple versions of a game's lua and switch which one Steam uses."
        actions={
          <>
            <button
              className="btn ghost"
              disabled={!selected || busy || canDownload === false}
              title={
                canDownload === false
                  ? "Downloading is unavailable right now — try again in a little while."
                  : undefined
              }
              onClick={async () => {
                if (!lua) return;
                await app.run(
                  () => api.startManifestDownload(lua.appId, lua.name),
                  "Manifest download started",
                );
              }}
            >
              <Download size={13} />
              Download
            </button>
            {onOpenLibrary && (
              <button className="btn ghost" onClick={onOpenLibrary}>
                <Library size={13} />
                Library
              </button>
            )}
            <button className="btn ghost" onClick={() => games.reload()}>
              <RefreshCw size={13} />
              Refresh
            </button>
          </>
        }
      />

      {!app.authenticated && (
        <div className="card pad">
          <div className="section-note">
            Verify to load depot details. Builds you already have still load from disk without a session.
          </div>
        </div>
      )}

      {canDownload === false && (
        <div className="card pad">
          <div className="section-note">
            Manifest downloads are unavailable right now. Depots, pins and saved builds still work.
          </div>
        </div>
      )}

      <div className="split-260">
        <section className="card pad picker">
          <SearchField value={query} onChange={setQuery} placeholder="Find a game in your library…" />
          <div className="game-list">
            {games.loading && (games.data ?? []).length === 0 ? (
              <div className="section-note" style={{ padding: 10 }}>
                Reading your library…
              </div>
            ) : filtered.length === 0 ? (
              <div className="section-note" style={{ padding: 10 }}>
                {query ? "No game matches that." : "No installed games were found."}
              </div>
            ) : (
              visible.map((game) => (
                <GameRow
                  key={game.appId}
                  game={game}
                  active={game.appId === selected}
                  onClick={() => setSelected(game.appId)}
                />
              ))
            )}
          </div>

          <div className="pager">
            <button
              className="icon-btn"
              disabled={current <= 0}
              onClick={() => setPage(current - 1)}
              aria-label="Previous page"
            >
              ‹
            </button>
            <span className="section-note">
              Page {current + 1} of {pages}
            </span>
            <button
              className="icon-btn"
              disabled={current >= pages - 1}
              onClick={() => setPage(current + 1)}
              aria-label="Next page"
            >
              ›
            </button>
            <div className="grow" />
            <label className="lib-per">
              Results per page
              <select value={perPage} onChange={(event) => setPerPage(Number(event.target.value))}>
                <option value={12}>12</option>
                <option value={30}>30</option>
                <option value={60}>60</option>
                <option value={0}>All</option>
              </select>
            </label>
          </div>
        </section>

        <div className="stack">
          {!selected ? (
            <div className="empty">
              <GitBranch />
              <div className="big">Pick a game</div>
              <div className="small">Its depots, pins and saved builds appear here.</div>
            </div>
          ) : loading && !lua ? (
            <div className="empty">
              <span className="spinner" />
              <div className="small">Reading the lua…</div>
            </div>
          ) : lua ? (
            <>
              <div className="build-title">
                <div>
                  <h2>{lua.name}</h2>
                  <div className="section-note">App {lua.appId}</div>
                </div>
                <div className="build-counts">
                  <span className="pill">{lua.depotCount} depots</span>
                  <span className="pill">{lua.dlcCount} DLC</span>
                  <span className={`pill ${lua.pinned ? "ok" : ""}`}>{lua.pinned} pinned</span>
                </div>
              </div>

              <section className="card pad">
                <SectionHead title="Active build">
                  <span className="section-note">
                    {lua.activeVariant ? "Saved build applied" : "Live lua"}
                  </span>
                </SectionHead>

                <select
                  className="select"
                  value={lua.activeVariant ?? ""}
                  disabled={busy || lua.variants.length === 0}
                  onChange={(event) => {
                    const id = event.target.value;
                    if (!id) return;
                    void mutate(() => api.buildsApplyVariant(lua.appId, id), "Build applied");
                  }}
                >
                  <option value="">Current lua (live)</option>
                  {lua.variants.map((variant) => (
                    <option key={variant.id} value={variant.id}>
                      {variant.name}
                    </option>
                  ))}
                </select>

                {renaming ? (
                  <div className="inline-field">
                    <input
                      autoFocus
                      value={renameText}
                      onChange={(event) => setRenameText(event.target.value)}
                      placeholder="Build name"
                    />
                    <button
                      className="btn primary small"
                      onClick={async () => {
                        if (!activeVariant) return;
                        const result = await mutate(
                          () => api.buildsRenameVariant(lua.appId, activeVariant.id, renameText),
                          "Build renamed",
                        );
                        if (result) setRenaming(false);
                      }}
                    >
                      Save
                    </button>
                    <button className="btn small ghost" onClick={() => setRenaming(false)}>
                      Cancel
                    </button>
                  </div>
                ) : naming ? (
                  <div className="inline-field">
                    <input
                      autoFocus
                      value={newName}
                      onChange={(event) => setNewName(event.target.value)}
                      placeholder="Name this build"
                    />
                    <button
                      className="btn primary small"
                      onClick={async () => {
                        const result = await mutate(
                          () => api.buildsSaveVariant(lua.appId, newName),
                          "Build saved",
                        );
                        if (result) {
                          setNaming(false);
                          setNewName("");
                        }
                      }}
                    >
                      Save
                    </button>
                    <button className="btn small ghost" onClick={() => setNaming(false)}>
                      Cancel
                    </button>
                  </div>
                ) : (
                  <div className="build-actions">
                    <button
                      className="btn primary"
                      disabled={busy || !lua.exists}
                      onClick={() =>
                        void mutate(() => api.buildsSaveVariant(lua.appId, ""), "Build saved")
                      }
                    >
                      <Bookmark size={13} />
                      Save as build
                    </button>
                    <button
                      className="btn ghost"
                      disabled={busy || !activeVariant}
                      onClick={() => {
                        if (!activeVariant) return;
                        setRenameText(activeVariant.name);
                        setRenaming(true);
                      }}
                    >
                      <Pencil size={13} />
                      Rename
                    </button>
                    <button
                      className="btn ghost"
                      disabled={!lua.exists}
                      onClick={() => {
                        setDraft(lua.text);
                        setEditing(true);
                      }}
                    >
                      Edit lua
                    </button>
                    <button
                      className="btn ghost danger"
                      disabled={busy || !activeVariant}
                      onClick={async () => {
                        if (!activeVariant) return;
                        await mutate(
                          () => api.buildsDeleteVariant(lua.appId, activeVariant.id),
                          "Build deleted",
                        );
                      }}
                    >
                      <Trash2 size={13} />
                      Delete
                    </button>
                  </div>
                )}
              </section>

              {editing ? (
                <section className="card pad">
                  <SectionHead title="Editor" />
                  <textarea
                    className="code"
                    value={draft}
                    spellCheck={false}
                    onChange={(event) => setDraft(event.target.value)}
                  />
                  <div className="build-actions end">
                    <button className="btn ghost" onClick={() => setEditing(false)}>
                      Cancel
                    </button>
                    <button
                      className="btn primary"
                      disabled={busy}
                      onClick={async () => {
                        const result = await mutate(
                          () => api.buildsSaveText(lua.appId, draft),
                          "lua saved",
                        );
                        if (result) {
                          setEditing(false);
                          games.reload();
                        }
                      }}
                    >
                      <Check size={13} />
                      Save lua
                    </button>
                  </div>
                </section>
              ) : (
                <section className="card pad">
                  <SectionHead title="Depots and DLC">
                    {info.loading && <span className="spinner" />}
                    <span className="section-note">{lua.rows.length} entries</span>
                  </SectionHead>

                  <SearchField value={rowQuery} onChange={setRowQuery} placeholder="Search depots, DLC, ids…" />

                  {!lua.exists ? (
                    <Empty
                      icon={<GitBranch />}
                      title="No lua for this game yet"
                      detail="Download a manifest or drop a .lua file to start, then its depots appear here."
                    />
                  ) : lua.rows.length === 0 ? (
                    <Empty
                      icon={<Search />}
                      title="This lua declares nothing"
                      detail="It has no addappid lines, so there is nothing to switch on."
                    />
                  ) : (
                    <>
                      {activeRows.length > 0 && (
                        <>
                          <div className="group-head">In this lua ({activeRows.length})</div>
                          <div className="build-list">
                            {activeRows.map((row) => (
                              <DepotRowView
                                key={`on-${row.id}`}
                                row={row}
                                depot={depots.get(row.id)}
                                onToggle={(field, value) =>
                                  void mutate(
                                    () => api.buildsSetEntry(lua.appId, row.id, field, value),
                                    field === "enabled"
                                      ? value
                                        ? "Depot enabled"
                                        : "Depot disabled"
                                      : value
                                        ? "Pin enabled"
                                        : "Pin disabled",
                                  )
                                }
                              />
                            ))}
                          </div>
                        </>
                      )}

                      {disabledRows.length > 0 && (
                        <>
                          <div className="group-head">Switched off ({disabledRows.length})</div>
                          <div className="build-list">
                            {disabledRows.map((row) => (
                              <DepotRowView
                                key={`off-${row.id}`}
                                row={row}
                                depot={depots.get(row.id)}
                                onToggle={(field, value) =>
                                  void mutate(
                                    () => api.buildsSetEntry(lua.appId, row.id, field, value),
                                    field === "enabled" ? "Depot enabled" : "Pin updated",
                                  )
                                }
                              />
                            ))}
                          </div>
                        </>
                      )}
                    </>
                  )}
                </section>
              )}
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
