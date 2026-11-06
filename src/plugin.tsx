import { useState } from "react";
import {
  CheckCircle2,
  Puzzle,
  RefreshCw,
  TriangleAlert,
  Wrench,
  XCircle,
} from "lucide-react";
import { api, toFailure, useAsync, type PluginState } from "./lib";
import { PageHead, type Route } from "./components";
import type { PageApi } from "./pages";

function ComponentRow({
  label,
  ok,
  warn,
  unknown,
  text,
}: {
  label: string;
  ok: boolean;
  warn?: boolean;
  /** Present but unconfirmed (the release feed was unreachable): neither a tick nor a warning. */
  unknown?: boolean;
  text: string;
}) {
  return (
    <div className="component">
      <div className="label">{label}</div>
      <div className="value">
        {ok ? (
          <CheckCircle2 size={14} style={{ color: "var(--ok)" }} />
        ) : warn ? (
          <TriangleAlert size={14} style={{ color: "var(--warn)" }} />
        ) : unknown ? (
          <CheckCircle2 size={14} style={{ color: "var(--muted)" }} />
        ) : (
          <XCircle size={14} style={{ color: "var(--muted)" }} />
        )}
        <span className={ok ? "on" : warn ? "warn" : ""}>{text}</span>
      </div>
    </div>
  );
}

export function PluginPage({
  app,
  onNavigate,
}: {
  app: PageApi;
  onNavigate: (route: Route) => void;
}) {
  const plugin = useAsync(() => api.pluginState(), []);
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [confirm, setConfirm] = useState<"install" | "remove" | null>(null);

  const state: PluginState | null = plugin.data;
  const installed = state?.installed ?? false;
  const updateAvailable = state?.updateAvailable ?? false;
  // Three loader states, not two: matching bytes, bytes that differ from a digest we actually read, and a
  // loader we could not check because the release feed is unreachable. The third must not wear "Out of
  // date" — an offline app should not push an update it cannot describe.
  const loaderVerified = Boolean(state?.dllVerified);
  const loaderOutOfDate = Boolean(state?.loader && loaderVerified && !state?.dllMatches);
  const needsWork = updateAvailable || loaderOutOfDate;

  async function install() {
    setConfirm(null);
    setBusy("install");
    try {
      const next = await api.pluginInstall();
      plugin.setData(next);
      app.notify("ok", "Plugin installed", "Start Steam to see it on store pages.");
    } catch (error) {
      app.notify("err", toFailure(error).message);
    } finally {
      setBusy(null);
    }
  }

  async function uninstall() {
    setConfirm(null);
    setBusy("remove");
    try {
      await api.pluginUninstall();
      plugin.reload();
      app.notify("ok", "Plugin removed");
    } catch (error) {
      app.notify("err", toFailure(error).message);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="page-inner narrow">
      <PageHead
        title="Steamware Plugin"
        subtitle="The Steamware Steam Plugin!"
        big
      />

      <section className="card pad plugin-hero">
        <div className="hero">
          <div className="hero-icon">
            <Puzzle size={22} />
          </div>
          <div className="grow">
            <h2 className="hero-title">{installed ? "Installed" : "Not installed"}</h2>
            {installed && (
              <div className="section-note">
                Version <strong>{state?.version ?? "—"}</strong>
              </div>
            )}
            <div className="section-note">
              Its menu inside Steam is painted in your accent — change it in Settings and open store pages
              repaint themselves.
            </div>
          </div>
          {installed && loaderVerified && !needsWork && <span className="pill ok">Up to date</span>}
          {needsWork && <span className="pill warn">Update available</span>}
        </div>

        <div className="divider" />

        <div className="components">
          <ComponentRow
            label="Frontend"
            ok={Boolean(state?.frontend)}
            text={state?.frontend ? "Installed" : "Not installed"}
          />
          <ComponentRow
            label="Loader (winmm.dll)"
            ok={Boolean(state?.loader && loaderVerified && state?.dllMatches)}
            warn={loaderOutOfDate}
            unknown={Boolean(state?.loader && !loaderVerified)}
            text={
              !state?.loader
                ? "Not installed"
                : !loaderVerified
                  ? "Installed"
                  : state.dllMatches
                    ? "Up to date"
                    : "Out of date"
            }
          />
        </div>

        {state?.millennium && (
          <div className="callout info">
            Millennium is installed too. Both can coexist; only one should inject the store page.
          </div>
        )}

        {state?.restartRequired && (
          <div className="callout info plugin-restart">
            <span className="grow">
              Steam is running without its plugin port. It only opens one while starting, so store pages stay
              untouched until Steam restarts once.
            </span>
            <button
              className="btn small"
              disabled={Boolean(busy)}
              onClick={() => app.run(() => api.steamRestart())}
            >
              Restart Steam
            </button>
          </div>
        )}

        <div className="hero-actions">
          {/* The main action keeps the brand gradient whether it reads Install, Update or Reinstall: the
              button does the same thing in all three states, so it should not change weight when it does. */}
          <button
            className="btn primary"
            disabled={Boolean(busy)}
            onClick={() => setConfirm("install")}
          >
            {busy === "install" && <span className="spinner" />}
            {!installed ? "Install" : needsWork ? "Update" : "Reinstall"}
          </button>
          {installed && (
            <button className="btn ghost" disabled={Boolean(busy)} onClick={() => setConfirm("remove")}>
              Uninstall
            </button>
          )}
          <div className="grow" />
          <button
            className="btn ghost"
            disabled={Boolean(busy)}
            onClick={() => plugin.reload()}
            title="Check for updates"
          >
            <RefreshCw size={13} />
            Check for updates
          </button>
        </div>

        {Boolean(busy) && <div className="progress-line" />}

        {state?.status && (
          <p className="section-note" style={{ marginTop: 10, color: "var(--warn)" }}>
            {state.status}
          </p>
        )}
      </section>

      <section className="card pad callout-row">
        <div className="callout-icon">
          <Wrench size={17} />
        </div>
        <div className="grow">
          <div className="title">Game fixes</div>
          <div className="meta">
            Online fixes and bypasses for the games you already own.
          </div>
        </div>
        <button className="btn" onClick={() => onNavigate("fixes")}>
          <Wrench size={13} />
          Browse fixes
        </button>
      </section>

      <p className="section-note" style={{ marginTop: 2 }}>
        The in-Steam overlay talks to a small local API. Keep Steamware running while you browse Steam, or
        the button can open but the actions behind it will not answer.
      </p>

      {confirm && (
        <div className="scrim" onClick={() => setConfirm(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <h2>{confirm === "install" ? "Install the plugin?" : "Remove the plugin?"}</h2>
            <p>
              {confirm === "install"
                ? "Steam will restart if the loader has to change. The frontend is written to your Steamware profile."
                : "This removes the frontend and the loader from Steam. Your library is untouched."}
            </p>
            <div className="actions">
              <button className="btn ghost" onClick={() => setConfirm(null)}>
                Cancel
              </button>
              <button
                className="btn primary"
                onClick={() => (confirm === "install" ? void install() : void uninstall())}
              >
                {confirm === "install" ? "Install" : "Remove"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
