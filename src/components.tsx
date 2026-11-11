import { useEffect, useState, type DragEvent, type ReactNode } from "react";
import type { ImportFile, InstallSummary } from "./lib";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  Download,
  ExternalLink,
  FolderOpen,
  GitBranch,
  Home,
  Inbox,
  Library,
  Minus,
  Puzzle,
  RefreshCw,
  Search,
  Settings as SettingsIcon,
  ShieldCheck,
  ShoppingBag,
  SlidersHorizontal,
  Square,
  Upload,
  Wrench,
  X,
} from "lucide-react";
import {
  api,
  toFailure,
  formatBytes,
  formatCompact,
  formatCount,
  coverCandidates,
  steamCapsule,
  useCover,
  useCoverChain,
  useGameStat,
  type DownloadItem,
  type FixGame,
  type InstalledGame,
  type SteamStatus,
  type StoreItem,
  type UpdateStatus,
} from "./lib";
import brandIcon from "./assets/steamware-icon.png";

export type Route =
  | "home"
  | "store"
  | "library"
  | "fixes"
  | "builds"
  | "mode"
  | "plugin"
  | "downloads"
  | "settings";

/* ── Shell chrome ─────────────────────────────────────────────────────────────────────────────── */

/// The product mark, from the same artwork the window and installer use (`icons/icon.ico`, rendered from
/// `resources/steamware-icon.png`). It is an <img> rather than a drawn shape so the title bar cannot drift
/// from the icon the OS shows for the running app.
export function BrandMark({ size = 16 }: { size?: number }) {
  return (
    <img
      className="brand-mark"
      src={brandIcon}
      alt=""
      width={size}
      height={size}
      draggable={false}
    />
  );
}

/// Discord's own mark (the Wumpus head, 24×24 viewBox from simple-icons), as the WPF build drew it — the
/// community pill should read as Discord at a glance instead of as a generic chat bubble.
const DISCORD_GLYPH =
  "M20.317 4.3698a19.7913 19.7913 0 00-4.8851-1.5152.0741.0741 0 00-.0785.0371c-.211.3753-.4447.8648-.6083 1.2495-1.8447-.2762-3.68-.2762-5.4868 0-.1636-.3933-.4058-.8742-.6177-1.2495a.077.077 0 00-.0785-.037 19.7363 19.7363 0 00-4.8852 1.515.0699.0699 0 00-.0321.0277C.5334 9.0458-.319 13.5799.0992 18.0578a.0824.0824 0 00.0312.0561c2.0528 1.5076 4.0413 2.4228 5.9929 3.0294a.0777.0777 0 00.0842-.0276c.4616-.6304.8731-1.2952 1.226-1.9942a.076.076 0 00-.0416-.1057c-.6528-.2476-1.2743-.5495-1.8722-.8923a.077.077 0 01-.0076-.1277c.1258-.0943.2517-.1923.3718-.2914a.0743.0743 0 01.0776-.0105c3.9278 1.7933 8.18 1.7933 12.0614 0a.0739.0739 0 01.0785.0095c.1202.099.246.1981.3728.2924a.077.077 0 01-.0066.1276 12.2986 12.2986 0 01-1.873.8914.0766.0766 0 00-.0407.1067c.3604.698.7719 1.3628 1.225 1.9932a.076.076 0 00.0842.0286c1.961-.6067 3.9495-1.5219 6.0023-3.0294a.077.077 0 00.0313-.0552c.5004-5.177-.8382-9.6739-3.5485-13.6604a.061.061 0 00-.0312-.0286zM8.02 15.3312c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9555-2.4189 2.157-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.9555 2.4189-2.1569 2.4189zm7.9748 0c-1.1825 0-2.1569-1.0857-2.1569-2.419 0-1.3332.9554-2.4189 2.1569-2.4189 1.2108 0 2.1757 1.0952 2.1568 2.419 0 1.3332-.946 2.4189-2.1568 2.4189Z";

/// The Discord pill: a quiet surface with Discord's blurple mark and label, exactly the HomeView button.
export function DiscordPill({ onClick }: { onClick: () => void }) {
  return (
    <button className="discord-pill" onClick={onClick} title="Join the Steamware Discord">
      <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
        <path fill="currentColor" d={DISCORD_GLYPH} />
      </svg>
      <span>Discord</span>
    </button>
  );
}

export function TitleBar() {
  return (
    <div className="titlebar" data-tauri-drag-region>
      <BrandMark size={16} />
      <span className="name" data-tauri-drag-region>
        Steamware
      </span>
      <div className="spacer" data-tauri-drag-region />
      <div className="win-controls">
        <button aria-label="Minimise" onClick={() => void api.windowMinimize()}>
          <Minus size={13} />
        </button>
        <button aria-label="Maximise" onClick={() => void api.windowToggleMaximize()}>
          <Square size={11} />
        </button>
        <button className="close" aria-label="Close" onClick={() => void api.windowClose()}>
          <X size={14} />
        </button>
      </div>
    </div>
  );
}

interface NavRailProps {
  route: Route;
  onNavigate: (route: Route) => void;
  steam: SteamStatus | null;
  activeDownloads: number;
  onSteamAction: () => void;
}

const MAIN_NAV: Array<{ route: Route; label: string; icon: ReactNode; soon?: boolean }> = [
  { route: "home", label: "Home", icon: <Home /> },
  { route: "store", label: "Store", icon: <ShoppingBag /> },
  { route: "library", label: "Library", icon: <Library /> },
  { route: "fixes", label: "Fixes", icon: <Wrench /> },
  { route: "builds", label: "Builds", icon: <GitBranch /> },
  { route: "mode", label: "Mode", icon: <SlidersHorizontal /> },
  { route: "plugin", label: "Plugin", icon: <Puzzle /> },
];

export function NavRail({ route, onNavigate, steam, activeDownloads, onSteamAction }: NavRailProps) {
  // "Restart" rather than "running": the button's job when Steam is open is to bounce it, which is what
  // makes Steam re-read the luas and manifests Steamware just installed.
  const steamLabel = steam?.running
    ? "Restart Steam"
    : steam?.installed
      ? "Launch Steam"
      : "Steam not found";
  const steamTitle = steam?.running
    ? "Restart Steam to load new luas and manifests"
    : (steam?.installPath ?? undefined);

  return (
    <nav className="rail" aria-label="Main">
      <div className="group">
        {MAIN_NAV.map((item) => (
          <button
            key={item.route}
            className={`nav-item ${route === item.route ? "active" : ""}`}
            onClick={() => onNavigate(item.route)}
            aria-current={route === item.route ? "page" : undefined}
          >
            {item.icon}
            <span>{item.label}</span>
            {item.soon && <span className="soon">soon</span>}
          </button>
        ))}
      </div>

      <div className="footer">
        <button className="nav-item" onClick={onSteamAction} title={steamTitle}>
          <span className={`status-dot ${steam?.running ? "on" : ""}`} style={{ marginLeft: 4, marginRight: 5 }} />
          <span>{steamLabel}</span>
        </button>
        <button
          className={`nav-item ${route === "downloads" ? "active" : ""}`}
          onClick={() => onNavigate("downloads")}
        >
          <Download />
          <span>Downloads</span>
          {activeDownloads > 0 && <span className="count">{activeDownloads}</span>}
        </button>
        <button
          className={`nav-item ${route === "settings" ? "active" : ""}`}
          onClick={() => onNavigate("settings")}
        >
          <SettingsIcon />
          <span>Settings</span>
        </button>
      </div>
    </nav>
  );
}

export function PageHead({
  title,
  subtitle,
  actions,
  big,
}: {
  title: string;
  subtitle?: ReactNode;
  actions?: ReactNode;
  /** The Plugin page's 24px title; every other page uses the 22px one. */
  big?: boolean;
}) {
  return (
    <header className={`page-head${big ? " big" : ""}`}>
      <div>
        <h1>{title}</h1>
        {subtitle && <p className="sub">{subtitle}</p>}
      </div>
      {actions && <div className="actions">{actions}</div>}
    </header>
  );
}

export function SectionHead({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="section-head">
      <h2>{title}</h2>
      {children && <div className="right">{children}</div>}
    </div>
  );
}

/* ── Data display ───────────────────────────────────────────────────────────────────────────── */

export function Stat({
  label,
  value,
  foot,
  accent,
  onClick,
}: {
  label: string;
  value: ReactNode;
  foot?: ReactNode;
  accent?: boolean;
  onClick?: () => void;
}) {
  const Tag = onClick ? "button" : "div";
  return (
    <Tag className="stat" onClick={onClick} type={onClick ? "button" : undefined}>
      <div className="label">{label}</div>
      <div className={`value ${accent ? "accent" : ""}`}>{value}</div>
      {foot && <div className="foot">{foot}</div>}
    </Tag>
  );
}

export function SearchField({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  return (
    <div className="search">
      <Search />
      <input
        type="text"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
        spellCheck={false}
      />
    </div>
  );
}

/**
 * The tone a review score is drawn in. Steam's own bands, on the numbers rather than on its wording: a game
 * almost nobody recommends is red, one that is merely divisive is amber, and a well-liked one is green.
 */
export function scoreTone(percent: number): string {
  if (percent < 40) return "bad";
  if (percent < 70) return "mixed";
  return "good";
}

/**
 * How long ago Steam was read for the numbers on a card, or nothing at all while they are current.
 *
 * A card normally carries a live figure. It carries an older one when Valve refused the refresh and the Worker
 * served the reading it already had — and a player count from twenty minutes ago must not be labelled as
 * something happening "right now".
 */
function readAge(readAt: number | undefined): string {
  if (!readAt) return "";
  const minutes = Math.floor((Date.now() - readAt) / 60_000);
  if (minutes < 5) return "";
  if (minutes < 120) return ` · read ${minutes} min ago`;
  return ` · read ${Math.floor(minutes / 60)} h ago`;
}

/**
 * The two numbers under a card's art: how many of the reviews recommend the game, and how many people are
 * playing it right now.
 *
 * While they are on their way the row is drawn at its final height out of two empty cells with a shimmer where
 * the figures will land, and says so. The card therefore never grows a row of text in one go, which is what
 * made a grid of twenty-four tiles visibly settle a second after it appeared.
 *
 * Whichever of the two is genuinely unknown is left out rather than drawn as a dash, and a game the Worker had
 * nothing for draws no row at all once that answer has arrived. The colours the values take come from CSS, so
 * they follow the palette like everything else on the card.
 */
export function StatsRow({ appId }: { appId?: string | number }) {
  const { stat, loading } = useGameStat(appId);

  if (!stat) {
    if (!loading || !appId) return null;
    return (
      <div className="tile-stats loading" role="status" aria-label="Loading stats">
        <div className="tile-stat ghost" aria-hidden="true">
          <div className="value skeleton" />
          <div className="key skeleton" />
        </div>
        <div className="tile-stat ghost" aria-hidden="true">
          <div className="value skeleton" />
          <div className="key skeleton" />
        </div>
        <span className="stats-loading">Loading stats</span>
      </div>
    );
  }

  const { percent, players, reviews, desc, readAt } = stat;
  if (percent === null && players === null) return null;

  // Steam hands the score over as a whole number ("85% of the 2,641,025 user reviews"), so it is drawn as
  // one: `85%` rather than a `85.00%` that promises two decimals the source never had. A two-decimal score
  // still prints with its decimals, so a better source can be dropped in without touching this.
  const score = percent === null ? "" : Number.isInteger(percent) ? `${percent}%` : `${percent.toFixed(2)}%`;

  return (
    <div className="tile-stats">
      {percent !== null && (
        <div
          className={`tile-stat score ${scoreTone(percent)}`}
          title={`${score} of ${formatCount(reviews)} reviews${desc ? ` · ${desc}` : ""}${readAge(readAt)}`}
        >
          <div className="value">{score}</div>
          <div className="key">{formatCompact(reviews)} reviews</div>
        </div>
      )}
      {players !== null && (
        <div className="tile-stat players" title={`${formatCount(players)} playing${readAge(readAt) || " right now"}`}>
          <div className="value">{formatCount(players)}</div>
          <div className="key">In-Game</div>
        </div>
      )}
    </div>
  );
}

export function ArtTile({
  appId,
  name,
  cover,
  sub,
  badge,
  capsule,
  onClick,
}: {
  appId?: string | number;
  name: string;
  cover?: string | null;
  sub?: string;
  badge?: string;
  capsule?: boolean;
  onClick?: () => void;
}) {
  const src = useCoverChain(appId ? coverCandidates(appId, cover) : cover ? [cover] : [], appId);
  return (
    <button className={`tile ${capsule ? "capsule" : ""}`} onClick={onClick} type="button" title={name}>
      <div className="art">
        {src ? <img src={src} alt="" loading="lazy" /> : <div className="fallback">{name.slice(0, 18)}</div>}
        {badge && <span className="badge">{badge}</span>}
      </div>
      <div className="name">{name}</div>
      <StatsRow appId={appId} />
      {sub && <div className="sub">{sub}</div>}
    </button>
  );
}

export function LibraryTile({ game, onClick }: { game: InstalledGame; onClick?: () => void }) {
  const src = useCoverChain(coverCandidates(game.appId), game.appId);
  const played = game.lastPlayed ? new Date(game.lastPlayed * 1000).toLocaleDateString() : "never";
  return (
    <button className="lib-card" type="button" onClick={onClick} title={game.name}>
      <div className="art">
        {src ? <img src={src} alt="" loading="lazy" /> : <div className="fallback">{game.name.slice(0, 18)}</div>}
      </div>
      <div className="lib-body">
        <div className="lib-name">{game.name}</div>
        <div className="lib-meta">App ID: {game.appId}</div>
        <div className="lib-meta">
          {formatBytes(game.sizeOnDisk)} · played {played}
        </div>
        <StatsRow appId={game.appId} />
      </div>
    </button>
  );
}

export function StoreTile({ item, onClick }: { item: StoreItem; onClick?: () => void }) {
  return <ArtTile appId={item.appid} name={item.name} cover={item.cover} onClick={onClick} />;
}

export function FixCard({ game, onDownload }: { game: FixGame; onDownload: (game: FixGame) => void }) {
  const fix = game.fixes[0];
  // Community fallback entries have a synthetic appid. Do not turn that into a broken Steam CDN URL.
  const src = useCover(
    game.capsuleImage || (/^\d+$/.test(game.appid) ? steamCapsule(game.appid) : null),
  );
  const source = game.source === "CrakFiles" ? "CrakFiles community" : game.source || "DepotBox";
  const tags = Array.from(new Set(game.fixes.flatMap((entry) => entry.tags.map((tag) => tag.toLowerCase()))));

  return (
    <article className="fix-card">
      <div className="art">
        {src ? <img src={src} alt="" loading="lazy" /> : <div className="fix-art-empty">{game.name.slice(0, 28)}</div>}
      </div>
      <div className="content">
        <div className="name" title={game.name}>
          {game.name}
        </div>
        <div className="fix-source" title="Source of this fix catalog entry">
          {source}
        </div>
        <div className="tags">
          {tags.slice(0, 3).map((tag) => (
            <span key={tag} className={`tag ${tag}`}>
              {tag}
            </span>
          ))}
          {tags.length === 0 && <span className="tag">fix</span>}
        </div>
        <div className="foot">
          <span className="size">{fix?.size ?? "—"}</span>
          <button className="btn small" type="button" onClick={() => onDownload(game)} disabled={!fix}>
            <Download size={12} />
            Download
          </button>
        </div>
      </div>
    </article>
  );
}

export function Empty({
  icon,
  title,
  detail,
  action,
}: {
  icon?: ReactNode;
  title: string;
  detail?: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      {icon ?? <Inbox />}
      <div className="big">{title}</div>
      {detail && <div className="small">{detail}</div>}
      {action && <div style={{ marginTop: 8 }}>{action}</div>}
    </div>
  );
}

export function SkeletonGrid({ count = 8, capsule }: { count?: number; capsule?: boolean }) {
  return (
    <div className="grid">
      {Array.from({ length: count }).map((_, index) => (
        <div className="tile" key={index}>
          <div className={`skeleton ${capsule ? "capsule" : ""}`} style={{ aspectRatio: capsule ? "231/87" : "460/215" }} />
          <div className="skeleton" style={{ height: 10, width: "70%" }} />
          {/* Room for the two stats pills, so a grid does not jump when the numbers land. */}
          <div className="skeleton" style={{ height: 34 }} />
        </div>
      ))}
    </div>
  );
}

/* ── Drop zone ──────────────────────────────────────────────────────────────────────────────── */

export function FileDropZone({
  onDone,
  title = "Drop .lua, .manifest or .zip files here",
  subtitle = "— or add them with Browse. Steam is told the game exists, not asked to download it.",
}: {
  onDone?: (summary: InstallSummary) => void;
  title?: string;
  subtitle?: string;
}) {
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function install(files: ImportFile[]) {
    if (files.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      onDone?.(await api.installFiles(files));
    } catch (failure) {
      setError(toFailure(failure).message);
    } finally {
      setBusy(false);
    }
  }

  async function browse() {
    try {
      const paths = await api.pickFiles();
      if (paths.length === 0) return;
      await install(paths.map((path) => ({ name: baseName(path), path })));
    } catch (failure) {
      setError(toFailure(failure).message);
    }
  }

  async function drop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setDragging(false);
    // A drop hands the page a `File` with no path on disk, so the bytes are read here and passed as base64.
    const dropped = [...event.dataTransfer.files];
    if (dropped.length === 0) return;
    const files = await Promise.all(
      dropped.map(async (file) => ({ name: file.name, data: await toBase64(file) })),
    );
    await install(files);
  }

  return (
    <div
      className={`dropzone${dragging ? " over" : ""}`}
      onDragOver={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={drop}
    >
      <div className="drop-title">{busy ? "Adding to Steam…" : title}</div>
      <div className="drop-hint">{error ?? subtitle}</div>
      <div className="link-row">
        <button onClick={browse} disabled={busy} type="button">
          <FolderOpen />
          Browse files…
        </button>
        {busy && <span className="spinner" />}
      </div>
    </div>
  );
}

/** The last segment of a path, whichever separator the platform used. */
function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** A dropped file's bytes as base64. Chunked, because one spread call over a large manifest overflows the stack. */
async function toBase64(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  }
  return btoa(binary);
}

/* ── Downloads ─────────────────────────────────────────────────────────────────────────────── */

export function DownloadRow({
  item,
  onCancel,
  onReveal,
  onInstall,
}: {
  item: DownloadItem;
  onCancel: () => void;
  onReveal: () => void;
  onInstall: () => void;
}) {
  const percent = item.total > 0 ? Math.min(100, (item.received / item.total) * 100) : 0;
  const active = item.status === "downloading" || item.status === "queued";

  const installed = (item.installedLua ?? 0) + (item.installedManifests ?? 0) > 0;
  const badge = {
    queued: <span className="pill">Queued</span>,
    downloading: <span className="pill">Downloading</span>,
    done: <span className={`pill ${installed ? "ok" : ""}`}>{installed ? "Added" : "Ready"}</span>,
    failed: <span className="pill err">Failed</span>,
    cancelled: <span className="pill warn">Cancelled</span>,
  }[item.status] ?? <span className="pill">{item.status}</span>;

  return (
    <div className="dl-row">
      <div style={{ minWidth: 0 }}>
        <div className="name" title={item.title}>
          {item.title}
        </div>
        <div className="meta">
          {item.kind === "fix" ? "Game fix" : "Manifest"}
          {item.total > 0 && ` · ${formatBytes(item.received)} of ${formatBytes(item.total)}`}
          {installed &&
            ` · ${item.installedLua} lua, ${item.installedManifests} manifest${item.installedManifests === 1 ? "" : "s"} in Steam`}
          {item.installError ? ` · saved, not installed: ${item.installError}` : ""}
          {item.error ? ` · ${item.error}` : ""}
        </div>
      </div>
      <div>
        <div className="progress">
          <div className="fill" style={{ width: `${item.status === "done" ? 100 : percent}%` }} />
        </div>
      </div>
      <div className="dl-status">
        {badge}
        {active ? (
          <button className="icon-btn" onClick={onCancel} title="Cancel" aria-label="Cancel">
            <X />
          </button>
        ) : item.status === "done" ? (
          <>
            <button className="icon-btn" onClick={onInstall} title="Install .lua files" aria-label="Install lua">
              <Upload />
            </button>
            <button className="icon-btn" onClick={onReveal} title="Show in folder" aria-label="Show in folder">
              <FolderOpen />
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}

/* ── Toasts ─────────────────────────────────────────────────────────────────────────────────── */

export type ToastKind = "info" | "ok" | "warn" | "err";

export interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
}

export function Toasts({ items, onDismiss }: { items: Toast[]; onDismiss: (id: number) => void }) {
  return (
    <div className="toasts" role="status" aria-live="polite">
      {items.map((toast) => (
        <div key={toast.id} className={`toast ${toast.kind}`}>
          <div className="bar" />
          <div className="text">
            <div>{toast.title}</div>
            {toast.detail && <div className="detail">{toast.detail}</div>}
          </div>
          <button className="icon-btn" onClick={() => onDismiss(toast.id)} aria-label="Dismiss">
            <X />
          </button>
        </div>
      ))}
    </div>
  );
}

/* ── Auth overlay ───────────────────────────────────────────────────────────────────────────── */

export type AuthState =
  | { state: "idle" }
  | { state: "pending"; url: string; message: string }
  | { state: "ok"; expiresAt: number }
  | { state: "failed"; message: string };

export function AuthOverlay({
  auth,
  onRetry,
  onDismiss,
  onOpen,
}: {
  auth: AuthState;
  onRetry: () => void;
  onDismiss: () => void;
  onOpen: (url: string) => void;
}) {
  if (auth.state === "idle" || auth.state === "ok") return null;
  const pending = auth.state === "pending";
  const url = auth.state === "pending" ? auth.url : "";

  return (
    <div className="scrim">
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="auth-title">
        <h2 id="auth-title">{pending ? "Checking that you're human" : "Verification needed"}</h2>
        <p>
          {pending
            ? "Your browser has opened a Cloudflare check. Finish it there and this window unlocks on its own — usually within a second."
            : auth.message}
        </p>
        {pending && url && (
          <>
            <div className="link">{url}</div>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="btn ghost" onClick={() => onOpen(url)}>
                <ExternalLink size={13} />
                Open again
              </button>
              <span className="spinner" style={{ marginLeft: 4, alignSelf: "center" }} />
              <span className="section-note" style={{ alignSelf: "center" }}>
                Waiting…
              </span>
            </div>
          </>
        )}
        <div className="actions">
          <button className="btn ghost" onClick={onDismiss}>
            Cancel
          </button>
          {!pending && (
            <button className="btn primary" onClick={onRetry}>
              <RefreshCw size={13} />
              Try again
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * The gate an out-of-date build stops at.
 *
 * It is not a dialog, because there is nothing behind it to get back to: while it is up the pages are never
 * mounted at all (see App). So its only job is to say which version is needed and get the user there — one
 * button when the installer can be fetched and run here, the release page when it cannot.
 */
export function UpdateGate({
  status,
  onRecheck,
}: {
  status: UpdateStatus;
  onRecheck: () => Promise<void>;
}) {
  const [progress, setProgress] = useState<{ received: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rechecking, setRechecking] = useState(false);

  useEffect(() => {
    const unlisten = listen<{ received: number; total: number }>("update:progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, []);

  const install = () => {
    setBusy(true);
    setError(null);
    // On success the app quits itself, so there is no "done" state to draw: only a failure comes back.
    api.updateInstall().catch((failure) => {
      setBusy(false);
      setProgress(null);
      setError(toFailure(failure).message);
    });
  };

  const openRelease = () => {
    if (status.releaseUrl) void api.openExternal(status.releaseUrl);
  };

  const recheck = () => {
    setRechecking(true);
    void onRecheck().finally(() => setRechecking(false));
  };

  const total = progress?.total ?? 0;
  const percent = progress && total > 0 ? Math.min(100, (progress.received / total) * 100) : null;

  return (
    <div className="update-gate">
      <div className="update-card" role="alertdialog" aria-modal="true" aria-labelledby="update-title">
        <span className="update-eyebrow">Update required</span>
        <h1 id="update-title">Update to v{status.latest} to continue</h1>
        <p className="update-lede">
          This copy is v{status.current}. Steamware only runs on the newest release, so this window stays shut
          until the update is installed. It takes a few seconds and keeps your settings, library and
          downloads.
        </p>

        <div className="update-versions">
          <span className="update-chip old">v{status.current}</span>
          <ArrowRight size={14} aria-hidden="true" />
          <span className="update-chip new">v{status.latest}</span>
        </div>

        {status.notes && (
          <div className="update-notes" tabIndex={0}>
            {status.notes}
          </div>
        )}

        {busy ? (
          <div className="update-progress">
            <div className="progress">
              <div className="fill" style={{ width: percent === null ? "30%" : `${percent}%` }} />
            </div>
            <span className="update-progress-note">
              {progress
                ? `Downloading the installer · ${formatBytes(progress.received)}${
                    total > 0 ? ` of ${formatBytes(total)}` : ""
                  }`
                : "Starting the download…"}
            </span>
            <span className="section-note">
              Steamware closes itself and opens again once the new version is installed.
            </span>
          </div>
        ) : (
          <>
            <div className="update-actions">
              {status.canInstall ? (
                <button className="btn primary" onClick={install}>
                  <Download size={13} />
                  Update now
                </button>
              ) : (
                <button className="btn primary" onClick={openRelease}>
                  <ExternalLink size={13} />
                  Get the installer
                </button>
              )}
              <button className="btn ghost" onClick={recheck} disabled={rechecking}>
                <RefreshCw size={13} />
                {rechecking ? "Checking…" : "Check again"}
              </button>
            </div>
            {status.canInstall ? (
              // The gate must never be a dead end: if Windows blocks the in-app install (antivirus, a
              // policy, a locked temp folder), the release page is still one click away.
              <p className="update-footnote">
                If the update does not start, {" "}
                <button className="linkish" onClick={openRelease}>
                  download the installer from GitHub
                </button>
                .
              </p>
            ) : (
              <p className="update-footnote">
                This copy was not put here by the Steamware installer, so the new setup has to be downloaded
                by hand: save it, run it, and delete the old program file when it is done.
              </p>
            )}
          </>
        )}

        {error && <div className="update-error">{error}</div>}
      </div>
    </div>
  );
}

export function SecurityNote({ compact }: { compact?: boolean }) {
  return (
    <div className="row" style={{ alignItems: "flex-start", background: "var(--surface-subtle)" }}>
      <ShieldCheck size={15} style={{ color: "var(--ok)", marginTop: 1 }} />
      <div className="grow">
        <div className="title">Private by design</div>
        <div className="meta" style={{ whiteSpace: "normal" }}>
          {compact
            ? "Downloads and searches are proxied, so the provider keys never reach this PC."
            : "Manifests, fixes, covers and search are proxied through Steamware. No provider key is present in this app, and none can be extracted from it."}
        </div>
      </div>
    </div>
  );
}

export const StatusIcon = {
  ok: CheckCircle2,
  warn: AlertTriangle,
};

/**
 * A Settings switch.
 *
 * `role="switch"` rather than a checkbox, so a screen reader announces it as a state rather than a choice,
 * and `label` repeats the sentence next to it — the visible text is outside the control.
 */
export function Toggle({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  disabled?: boolean;
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
