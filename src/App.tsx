import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  api,
  applyTheme,
  toFailure,
  useDownloads,
  type Bootstrap,
  type ServerHealth,
  type SessionInfo,
  type Settings,
  type UpdateStatus,
} from "./lib";
import {
  AuthOverlay,
  NavRail,
  TitleBar,
  Toasts,
  UpdateGate,
  type AuthState,
  type Route,
  type Toast,
  type ToastKind,
} from "./components";
import {
  DownloadsPage,
  FixesPage,
  HomePage,
  LibraryPage,
  Onboarding,
  SettingsPage,
  type PageApi,
} from "./pages";
import { StorePage } from "./store";
import { BuildsPage } from "./builds";
import { ModePage } from "./modes";
import { PluginPage } from "./plugin";
import "./styles.css";

const SESSION_GRACE_SECONDS = 5;

export default function App() {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);
  const [route, setRoute] = useState<Route>("home");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [auth, setAuth] = useState<AuthState>({ state: "idle" });
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [health, setHealth] = useState<ServerHealth | null>(null);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [fixesFocus, setFixesFocus] = useState<{ appId: string; at: number } | null>(null);
  // "Manage build" in a library game's menu lands on the Builds page already showing that game.
  const [buildsFocus, setBuildsFocus] = useState<{ appId: string; at: number } | null>(null);
  const toastId = useRef(0);
  const downloads = useDownloads();

  /* ── Boot ───────────────────────────────────────────────────────────────────────────────── */

  useEffect(() => {
    let alive = true;
    api
      .bootstrap()
      .then((data) => {
        if (!alive) return;
        setBoot(data);
        setSession(data.session);
      })
      .catch((error) => alive && setFatal(toFailure(error).message));
    return () => {
      alive = false;
    };
  }, []);

  /* ── Update gate ─────────────────────────────────────────────────────────────────────────── */

  // Cached state only, so polling costs nothing. Rust does the talking to GitHub on its own clock; this
  // exists so a release published mid-session is caught without the user having to restart.
  useEffect(() => {
    let alive = true;
    const check = () => {
      void api
        .updateStatus()
        .then((next) => alive && setUpdate(next))
        .catch(() => {
          // Local state, so this only fails if the command is missing; whatever was known stays known.
        });
    };
    check();
    const timer = window.setInterval(check, 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const recheckUpdate = useCallback(async () => {
    try {
      setUpdate(await api.updateCheck());
    } catch {
      // The gate keeps saying what it last knew, which is the right answer for a failed check.
    }
  }, []);

  const notify = useCallback((kind: ToastKind, title: string, detail?: string) => {
    const id = (toastId.current += 1);
    setToasts((previous) => [...previous.slice(-3), { id, kind, title, detail }]);
    window.setTimeout(() => {
      setToasts((previous) => previous.filter((toast) => toast.id !== id));
    }, 4600);
  }, []);

  /* ── Theme ──────────────────────────────────────────────────────────────────────────────── */

  useEffect(() => {
    if (boot) applyTheme(boot.settings.accent, boot.settings.mode);
  }, [boot]);

  /* ── Session ─────────────────────────────────────────────────────────────────────────────── */

  const refreshSession = useCallback(async () => {
    try {
      setSession(await api.authStatus());
    } catch {
      setSession(null);
    }
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => void refreshSession(), 30_000);
    return () => window.clearInterval(timer);
  }, [refreshSession]);

  // The store-page plugin's Settings / Fix buttons land here. The app window is already raised by Rust;
  // this is what it navigates to, carrying the app id when the Fixes page should open on one game.
  useEffect(() => {
    const unlisten = listen<{ route?: string; appId?: string }>("plugin:navigate", (event) => {
      const target = event.payload.route;
      if (
        !target ||
        !["home", "store", "library", "fixes", "builds", "mode", "plugin", "downloads", "settings"].includes(
          target,
        )
      ) {
        return;
      }
      setRoute(target as Route);
      if (target === "fixes" && event.payload.appId) {
        setFixesFocus({ appId: event.payload.appId, at: Date.now() });
      }
    });
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, []);

  /* ── Server budget ──────────────────────────────────────────────────────────────────────── */

  // Read every fifteen seconds, and again whenever the page changes: the failure this exists for arrives
  // mid-session, when a page asks for something the server has no budget left to answer.
  useEffect(() => {
    let alive = true;
    const check = () => {
      void api
        .serverHealth()
        .then((next) => alive && setHealth(next))
        .catch(() => {
          // Local state, so this only fails if the command is missing; the banner stays as it was.
        });
    };
    check();
    const timer = window.setInterval(check, 15_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [route]);

  // What the window is showing, for Discord Rich Presence. The presence thread turns this key into the card's
  // first line, so the profile follows navigation without the two ever being able to disagree.
  useEffect(() => {
    void api.setPresencePage(route).catch(() => {
      // Presence is a nicety: a failed report must never show up as an error.
    });
  }, [route]);

  // A finished download speaks up once, the way the WPF build did: "X was added to Steam", or why it was
  // only saved. Without this the archive lands silently and the user never learns it is already playable.
  useEffect(() => {
    const unlisten = listen<{
      title: string;
      installedLua: number;
      installedManifests: number;
      installError: string | null;
      installLive: boolean;
      installLibrary: boolean;
    }>("download:done", (event) => {
      const { title, installedLua, installedManifests, installError, installLive, installLibrary } =
        event.payload;
      if (installError) {
        notify("warn", `${title} was downloaded`, installError);
        return;
      }
      if ((installedLua ?? 0) + (installedManifests ?? 0) > 0) {
        const manifests = `${installedManifests} manifest${installedManifests === 1 ? "" : "s"} installed`;
        notify(
          "ok",
          `${title} was added to Steam`,
          installLibrary
            ? `${manifests} — it is in your Steam library now. Finish the install from Steam when you want it on disk.`
            : installLive
              ? `${manifests} — it is in your library now.`
              : `${manifests}. Restart Steam once so it reads them.`,
        );
      } else {
        notify("ok", `${title} was downloaded`);
      }
    });
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, [notify]);

  useEffect(() => {
    const unlisten = listen<{ state: string; url?: string; message?: string; expiresAt?: number }>(
      "auth:status",
      (event) => {
        const payload = event.payload;
        if (payload.state === "pending") {
          setAuth({ state: "pending", url: payload.url ?? "", message: payload.message ?? "" });
        } else if (payload.state === "ok") {
          setAuth({ state: "ok", expiresAt: payload.expiresAt ?? 0 });
          void refreshSession();
          window.setTimeout(() => {
            setAuth((current) => (current.state === "ok" ? { state: "idle" } : current));
          }, 1100);
        } else {
          setAuth({ state: "failed", message: payload.message ?? "Verification failed." });
        }
      },
    );
    return () => {
      unlisten.then((dispose) => dispose());
    };
  }, [refreshSession]);

  const beginAuth = useCallback(() => {
    setAuth({ state: "pending", url: "", message: "Opening your browser…" });
    api.authBegin().catch((error) => setAuth({ state: "failed", message: toFailure(error).message }));
  }, []);

  const run = useCallback(
    async <T,>(task: () => Promise<T>, success?: string): Promise<T | null> => {
      try {
        const value = await task();
        if (success) notify("ok", success);
        return value;
      } catch (error) {
        const failure = toFailure(error);
        if (failure.code === "session_required" || failure.code === "session_expired") {
          notify("warn", "Verification needed", "Pass the quick check to continue.");
          beginAuth();
          return null;
        }
        notify("err", failure.message);
        return null;
      }
    },
    [beginAuth, notify],
  );

  const authenticated = Boolean(
    session && session.expiresAt > Math.floor(Date.now() / 1000) + SESSION_GRACE_SECONDS,
  );

  const pageApi: PageApi = useMemo(
    () => ({ notify, run, beginAuth, authenticated, session, version: boot?.version ?? "1.0.3" }),
    [notify, run, beginAuth, authenticated, session, boot?.version],
  );

  const activeDownloads = downloads.filter(
    (item) => item.status === "downloading" || item.status === "queued",
  ).length;

  const refreshSteam = useCallback(async () => {
    const steam = await run(() => api.steamRefresh());
    if (steam && boot) {
      setBoot({ ...boot, steam });
      notify(
        steam.installed ? "ok" : "warn",
        steam.running ? "Steam is running" : steam.installed ? "Steam detected" : "Steam not found",
        steam.installPath ?? undefined,
      );
    }
  }, [boot, notify, run]);

  // Steam's state is polled, not cached at boot: opening or closing Steam has to move the footer button
  // without the user thinking to press Refresh. The comparison keeps an unchanged status from re-rendering
  // the whole tree every few seconds.
  useEffect(() => {
    if (!boot) return;
    const timer = window.setInterval(async () => {
      try {
        const next = await api.steamRefresh();
        setBoot((previous) => {
          if (!previous) return previous;
          const current = previous.steam;
          const same =
            current.running === next.running &&
            current.installed === next.installed &&
            current.installPath === next.installPath &&
            current.gameCount === next.gameCount &&
            current.ownedCount === next.ownedCount &&
            current.libraryCount === next.libraryCount;
          return same ? previous : { ...previous, steam: next };
        });
      } catch {
        // A failed poll is not worth a toast: the next one will either work or the page will say so.
      }
    }, 4000);
    return () => window.clearInterval(timer);
  }, [boot?.settings.steamPath, Boolean(boot)]);

  const handleSteamAction = useCallback(async () => {
    if (!boot) return;
    if (!boot.steam.installed) {
      await refreshSteam();
      return;
    }
    if (boot.steam.running) {
      const restarted = await run(() => api.steamRestart(), "Restarting Steam… it reloads luas when it opens");
      if (restarted !== null) window.setTimeout(() => void refreshSteam(), 3000);
      return;
    }
    const started = await run(() => api.steamLaunch(), "Starting Steam…");
    if (started !== null) window.setTimeout(() => void refreshSteam(), 6000);
  }, [boot, refreshSteam, run]);

  function onSettingsSaved(settings: Settings) {
    // Only a new server address invalidates the session. Changing the accent or the download folder used to
    // drop the app back to "not verified" for as long as the next poll took, which looked like a logout.
    const serverChanged = boot ? boot.settings.apiBase !== settings.apiBase : false;
    setBoot((previous) => (previous ? { ...previous, settings } : previous));
    if (serverChanged) setSession(null);
    applyTheme(settings.accent, settings.mode);
    if (route === "home") setRoute("home");
  }

  /* ── Render ─────────────────────────────────────────────────────────────────────────────── */

  if (fatal) {
    return (
      <div className="app">
        <div className="scrim">
          <div className="modal">
            <h2>Steamware could not start</h2>
            <p>{fatal}</p>
          </div>
        </div>
      </div>
    );
  }

  if (!boot) {
    return (
      <div className="app">
        <TitleBar />
        <div className="body">
          <div className="page">
            <div style={{ display: "flex", alignItems: "center", gap: 10, paddingTop: 24 }}>
              <span className="spinner" />
              <span className="section-note">Starting Steamware…</span>
            </div>
          </div>
        </div>
      </div>
    );
  }

  // A build that is no longer current stops here. The pages are never mounted, so there is no way past the
  // gate — not by clicking, not by a keyboard shortcut, not by a stale navigation event — which is what
  // "not allowed to use it" has to mean. The window still closes normally.
  if (update?.required) {
    return (
      <div className="app">
        <TitleBar />
        <UpdateGate status={update} onRecheck={recheckUpdate} />
      </div>
    );
  }

  const needsOnboarding = !boot.settings.completedOnboarding || !boot.settings.apiBase;

  return (
    <div className="app">
      <TitleBar />
      <ServerBanner health={health} />
      <div className="body">
        <NavRail
          route={route}
          onNavigate={setRoute}
          steam={boot.steam}
          activeDownloads={activeDownloads}
          onSteamAction={() => void handleSteamAction()}
        />
        <main className="page" key={route}>
          {route === "home" && (
            <HomePage
              app={pageApi}
              boot={boot}
              steam={boot.steam}
              downloads={downloads}
              onNavigate={setRoute}
              onRefreshSteam={() => void refreshSteam()}
            />
          )}
          {route === "store" && <StorePage app={pageApi} />}
          {route === "library" && (
            <LibraryPage
              app={pageApi}
              steam={boot.steam}
              onRefresh={() => void refreshSteam()}
              onManageBuild={(appId) => {
                setBuildsFocus({ appId, at: Date.now() });
                setRoute("builds");
              }}
            />
          )}
          {route === "fixes" && <FixesPage app={pageApi} focus={fixesFocus} />}
          {route === "downloads" && (
            <DownloadsPage app={pageApi} items={downloads} />
          )}
          {route === "settings" && <SettingsPage app={pageApi} boot={boot} onSaved={onSettingsSaved} />}
          {route === "builds" && (
            <BuildsPage
              app={pageApi}
              onOpenLibrary={() => setRoute("library")}
              focus={buildsFocus}
            />
          )}
          {route === "mode" && <ModePage app={pageApi} />}
          {route === "plugin" && <PluginPage app={pageApi} onNavigate={setRoute} />}
        </main>
      </div>

      <Toasts
        items={toasts}
        onDismiss={(id) => setToasts((previous) => previous.filter((toast) => toast.id !== id))}
      />

      <AuthOverlay
        auth={auth}
        onRetry={beginAuth}
        onDismiss={() => setAuth({ state: "idle" })}
        onOpen={(url) => {
          if (url) void api.openExternal(url);
        }}
      />

      {needsOnboarding && (
        <Onboarding boot={boot} app={pageApi} onFinished={(settings) => setBoot({ ...boot, settings })} />
      )}
    </div>
  );
}

/**
 * One quiet line while the server has no requests to give.
 *
 * Everything that matters is already on this PC — the library, settings, builds, running downloads — and
 * covers come straight from Valve, so this is an explanation rather than an error: it says what still
 * works and when the rest comes back, and it disappears by itself when the hold lifts.
 */
function ServerBanner({ health }: { health: ServerHealth | null }) {
  if (!health?.degraded || !health.message) return null;
  const resumes =
    health.until > 0
      ? new Date(health.until * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
      : "";
  return (
    <div className="server-banner" role="status">
      <span className="server-banner-dot" aria-hidden="true" />
      <div className="server-banner-copy">
        <span className="server-banner-title">Working offline</span>
        <span className="server-banner-text">{health.message}</span>
      </div>
      {resumes && <span className="server-banner-until">Back around {resumes}</span>}
    </div>
  );
}
