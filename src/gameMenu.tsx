import { useEffect, useState } from "react";
import {
  Copy,
  Download,
  ExternalLink,
  FolderOpen,
  Layers,
  Loader2,
  Play,
  Store,
  Trash2,
  X,
} from "lucide-react";
import {
  api,
  coverCandidates,
  formatBytes,
  useAsync,
  useCoverChain,
  type OwnedGame,
  type StoreApp,
} from "./lib";
import type { PageApi } from "./pages";

/**
 * One game's menu, opened by clicking its card in the library.
 *
 * This is the old Windows build's detail flyout, rebuilt: a panel that slides in from the right with the
 * artwork, the title, the App ID (click to copy) and every action this build can carry as a full-width row.
 * The 2×2 block of secondary actions is the same four the WPF menu had — store page, open in Steam, reveal
 * the lua, copy the App ID.
 *
 * Nothing here needs the server. The cover comes from Valve's CDN, the facts come from the lua Steam is
 * loading, and the blurb is simply left out when there is no verified session to fetch it, so opening a game
 * while Steamware is offline still gives a complete, working menu.
 */
export function GameMenu({
  game,
  app,
  onClose,
  onManageBuild,
  onRemoved,
}: {
  game: OwnedGame;
  app: PageApi;
  onClose: () => void;
  onManageBuild: () => void;
  onRemoved: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [copying, setCopying] = useState(false);
  const cover = useCoverChain(coverCandidates(game.appId), game.appId);
  // A nicety, not a requirement: this read needs a verified session, and a failed one leaves the About block
  // out rather than turning the menu into an error.
  const details = useAsync<StoreApp>(() => api.storeApp(game.appId), [game.appId], {
    skip: !app.authenticated,
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const added = game.addedAt ? new Date(game.addedAt * 1000).toLocaleDateString() : "";
  const open = (url: string) => void app.run(() => api.openExternal(url));

  async function copy() {
    setCopying(true);
    const copied = await copyText(game.appId);
    setCopying(false);
    if (copied) app.notify("ok", "App ID copied", game.appId);
    else app.notify("err", "Could not copy", "Select the App ID and press Ctrl+C.");
  }

  async function remove() {
    setRemoving(true);
    // No canned success line: one game has two records, and only the result can say which went.
    const result = await app.run(() => api.libraryRemove(game.appId));
    setRemoving(false);
    if (result === null) return;
    onRemoved();
    if (result.stillInstalled) {
      app.notify(
        "warn",
        "Removed from Steamware",
        "Steam still has this game on disk, so it stays in its library. Uninstall it there to clear the tile.",
      );
    } else if (result.library) {
      app.notify(
        "ok",
        "Removed from both libraries",
        result.steamRunning ? "Restart Steam to clear its tile." : undefined,
      );
    } else {
      app.notify("ok", "Removed from your library");
    }
  }

  return (
    <div className="game-menu-layer" role="dialog" aria-modal="true" aria-label={`${game.name} menu`}>
      <div className="game-menu-scrim" onClick={onClose} />
      <aside className="game-menu">
        <header className="game-menu-head">
          <span className="game-menu-kicker">Game menu</span>
          <button className="icon-btn" onClick={onClose} aria-label="Close game menu">
            <X size={15} />
          </button>
        </header>

        <div className="game-menu-scroll">
          <div className="game-menu-art">
            {cover ? (
              <img src={cover} alt="" />
            ) : (
              <div className="game-menu-art-empty">{game.name}</div>
            )}
            {game.installed && <span className="game-menu-badge">Installed</span>}
          </div>

          <h2 className="game-menu-title">{game.name}</h2>

          <button className="game-menu-id" onClick={copy} title="Copy App ID">
            <span>
              App ID <span className="mono">{game.appId}</span>
            </span>
            {copying ? <Loader2 size={12} className="spin" /> : <Copy size={12} />}
          </button>

          <div className="game-menu-meta">
            <span>{game.installed ? formatBytes(game.sizeOnDisk) : "Not downloaded"}</span>
            <span>
              {game.depotCount} depot{game.depotCount === 1 ? "" : "s"} pinned
            </span>
            {added && <span>Added {added}</span>}
          </div>

          {details.data && (
            <section className="game-menu-about">
              <h3>About</h3>
              {details.data.shortDescription && <p>{details.data.shortDescription}</p>}
              <div className="game-menu-facts">
                {details.data.developers.length > 0 && (
                  <>
                    <span className="k">Developer</span>
                    <span className="v">{details.data.developers.join(", ")}</span>
                  </>
                )}
              </div>
              {details.data.genres.length > 0 && (
                <div className="game-menu-tags">
                  {details.data.genres.slice(0, 6).map((genre) => (
                    <span key={genre} className="game-menu-tag">
                      {genre}
                    </span>
                  ))}
                </div>
              )}
            </section>
          )}
        </div>

        <div className="game-menu-actions">
          {game.installed && (
            <button
              className="btn primary"
              onClick={() => open(`steam://rungameid/${game.appId}`)}
              title="Start this game through Steam"
            >
              <Play size={14} />
              Play
            </button>
          )}
          <button className="btn primary" onClick={onManageBuild}>
            <Layers size={14} />
            Manage build
          </button>
          <button
            className="btn ghost"
            onClick={() =>
              void app.run(
                () => api.startManifestDownload(game.appId, game.name),
                "Fetching — it installs into Steam when it lands",
              )
            }
            title="Download this game's manifests again and install them into Steam"
          >
            <Download size={14} />
            Update
          </button>

          <div className="game-menu-grid">
            <button className="btn ghost" onClick={() => open(`steam://store/${game.appId}`)}>
              <Store size={14} />
              Store page
            </button>
            <button className="btn ghost" onClick={() => open(`steam://nav/games/details/${game.appId}`)}>
              <ExternalLink size={14} />
              Open in Steam
            </button>
            <button className="btn ghost" onClick={() => void app.run(() => api.revealPath(game.luaPath))}>
              <FolderOpen size={14} />
              Reveal file
            </button>
            <button className="btn ghost" onClick={copy}>
              <Copy size={14} />
              Copy App ID
            </button>
          </div>

          {confirming ? (
            <div className="game-menu-confirm">
              <p>
                Remove <strong>{game.name}</strong> from both libraries? This deletes the lua Steamware lists
                and the entry it added to Steam
                {game.installed ? ", though Steam keeps its own record while the game is on disk" : ""}. The
                game&apos;s files are left alone.
              </p>
              <div className="game-menu-confirm-actions">
                <button className="btn ghost small" onClick={() => setConfirming(false)} disabled={removing}>
                  Cancel
                </button>
                <button className="btn danger small" onClick={remove} disabled={removing}>
                  {removing ? <Loader2 size={12} className="spin" /> : <Trash2 size={12} />}
                  Remove
                </button>
              </div>
            </div>
          ) : (
            <button className="btn danger game-menu-remove" onClick={() => setConfirming(true)}>
              <Trash2 size={14} />
              Remove from library
            </button>
          )}
        </div>
      </aside>
    </div>
  );
}

/**
 * Copy through the webview's clipboard, with the old selection fallback for a shell that refuses the async
 * API. Returns whether the text actually reached the clipboard, so the caller can say so.
 */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const copied = document.execCommand("copy");
      document.body.removeChild(area);
      return copied;
    } catch {
      return false;
    }
  }
}
