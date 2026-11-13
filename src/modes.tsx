import { useState } from "react";
import { Cloud, RefreshCw, Star, TriangleAlert, Wrench } from "lucide-react";
import { api, toFailure, useAsync, type ModeCard, type ModesState } from "./lib";
import { PageHead, SectionHead } from "./components";
import type { PageApi } from "./pages";

function actionLabel(card: ModeCard): string | null {
  if (!card.supported) return null;
  if (!card.managed) return card.active ? null : "Switch to this";
  if (!card.active) return "Switch to this";
  if (!card.installed) return "Install";
  if (card.updateAvailable) return "Update";
  return "Reinstall";
}

function ModeBadges({ card }: { card: ModeCard }) {
  return (
    <span className="badge-row">
      {card.recommended && (
        <span className="pill info">
          <Star size={10} />
          Recommended
        </span>
      )}
      {card.experimental && (
        <span className="pill warn">
          <TriangleAlert size={10} />
          Experimental
        </span>
      )}
    </span>
  );
}

export function ModePage({ app }: { app: PageApi }) {
  const modes = useAsync(() => api.modesState(), []);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ModeCard | null>(null);

  const state: ModesState | null = modes.data;
  const cards = state?.cards ?? [];
  const active = cards.find((card) => card.active) ?? null;
  const others = cards.filter((card) => !card.active);

  async function install(card: ModeCard) {
    setConfirm(null);
    setBusy(card.id);
    try {
      const next = await api.modesInstall(card.id);
      modes.setData(next);
      app.notify(
        "ok",
        card.active ? `${card.title} updated` : `${card.title} is now active`,
        card.managed ? "Steam was restarted so the new files load." : "Nothing was written to Steam.",
      );
    } catch (error) {
      app.notify("err", toFailure(error).message);
    } finally {
      setBusy(null);
    }
  }

  async function cloud(enable: boolean, update: boolean) {
    setBusy("cloud");
    try {
      const next = await api.modesCloudRedirect(enable, update);
      modes.setData(next);
      app.notify(
        "ok",
        update ? "CloudRedirect updated" : enable ? "CloudRedirect enabled" : "CloudRedirect disabled",
        "Takes effect the next time Steam starts.",
      );
    } catch (error) {
      app.notify("err", toFailure(error).message);
    } finally {
      setBusy(null);
    }
  }

  function actionButton(card: ModeCard, variant: "primary" | "ghost") {
    const label = actionLabel(card);
    if (!label) return null;
    const working = busy === card.id;
    return (
      <button
        className={`btn ${variant}`}
        disabled={Boolean(busy)}
        onClick={() => {
          if (!card.managed) void install(card);
          else setConfirm(card);
        }}
      >
        {working && <span className="spinner" />}
        {label}
      </button>
    );
  }

  return (
    <div className="page-inner mode-page">
      <PageHead
        title="Mode"
        subtitle="One unlocker at a time. Switching swaps the same three files in Steam's root."
        actions={
          <button className="btn ghost" disabled={Boolean(busy) || modes.loading} onClick={() => modes.reload()}>
            <RefreshCw size={13} />
            Check for updates
          </button>
        }
      />

      {!state?.steamPath && (
        <div className="mode-banner">Steam was not found. Set its folder in Settings before installing a mode.</div>
      )}

      {modes.error && <div className="mode-banner warn">{modes.error.message}</div>}

      <div className="mode-grid">
        <div className="mode-main">
          <section className="mode-current" aria-label="Active unlocker">
            {active ? (
              <>
                <div className="mode-current-head">
                  <Wrench size={16} />
                  <h2>{active.title}</h2>
                  {active.version && <span className="mode-version">{active.version}</span>}
                  <ModeBadges card={active} />
                  <div className="mode-current-action">{actionButton(active, "primary")}</div>
                </div>
                <p className="mode-desc">{active.description}</p>
                <div className="mode-current-foot">
                  <span className={`mode-state ${active.installed && !active.updateAvailable ? "ok" : "warn"}`}>
                    <span className="mode-state-dot" />
                    {busy === active.id ? "Working…" : active.status}
                  </span>
                </div>
              </>
            ) : (
              <div className="mode-empty">
                <span>No unlocker is active.</span>
                <span className="mode-empty-note">Pick one below — nothing is written to Steam until you do.</span>
              </div>
            )}
          </section>

          {Boolean(busy) && <div className="progress-line" />}

          {others.length > 0 && (
            <section className="mode-switch">
              <SectionHead title="Switch to" />
              <div className="mode-switch-list">
                {others.map((card) => (
                  <div className="mode-switch-row" key={card.id}>
                    <div className="mode-switch-text">
                      <div className="mode-switch-name">
                        {card.title}
                        <ModeBadges card={card} />
                      </div>
                      <p className="mode-desc">{card.description}</p>
                      {card.managed && <p className="mode-status">{card.status}</p>}
                    </div>
                    {actionButton(card, "ghost")}
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>

        <aside className="mode-aside">
          <section className={`mode-cloud ${state?.cloud.unlocked ? "" : "locked"}`}>
            <div className="mode-cloud-name">
              <Cloud size={14} />
              Cloud saves
            </div>
            <p className="mode-desc">
              Redirects Steam's cloud saves to a local folder, so restoring a build cannot overwrite them.
            </p>
            <p className="mode-status">{state?.cloud.status ?? "Checking…"}</p>
            <div className="mode-cloud-actions">
              <button
                className="btn"
                disabled={Boolean(busy) || !state?.cloud.unlocked}
                onClick={() => void cloud(!state?.cloud.enabled, false)}
              >
                {state?.cloud.enabled ? "Turn off" : "Turn on"}
              </button>
              {state?.cloud.updateAvailable && (
                <button
                  className="btn ghost"
                  disabled={Boolean(busy) || !state?.cloud.unlocked}
                  onClick={() => void cloud(state?.cloud.enabled ?? true, true)}
                >
                  <RefreshCw size={13} />
                  Update
                </button>
              )}
            </div>
          </section>

          <section className="mode-writes">
            <SectionHead title="What Steamware writes" />
            <p className="mode-writes-files">dwmapi.dll, xinput1_4.dll, OpenSteamTool.dll</p>
            <p className="mode-writes-note">
              One lua path entry beside them. Each archive is checked against its published SHA-256 before a byte
              is written.
            </p>
          </section>
        </aside>
      </div>

      {confirm && (
        <div className="scrim" onClick={() => setConfirm(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <h2>
              {confirm.active ? "Reinstall" : "Switch to"} {confirm.title}?
            </h2>
            <p>
              Steam will close while the files are replaced, then reopen on its own. Your library and saves are
              not touched.
            </p>
            <div className="actions">
              <button className="btn ghost" onClick={() => setConfirm(null)}>
                Cancel
              </button>
              <button className="btn primary" onClick={() => void install(confirm)}>
                Close Steam and continue
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
