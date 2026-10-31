import { useState } from "react";
import {
  Check,
  Cloud,
  ArrowRight,
  RefreshCw,
  ShieldCheck,
  Star,
  TriangleAlert,
  Wrench,
} from "lucide-react";
import { api, toFailure, useAsync, type ModeCard, type ModesState } from "./lib";
import { PageHead, SectionHead } from "./components";
import type { PageApi } from "./pages";

function actionLabel(card: ModeCard): string | null {
  if (!card.managed) return card.active ? null : "Switch to this";
  if (!card.active) return "Switch to this";
  if (!card.installed) return "Install";
  if (card.updateAvailable) return "Update";
  return "Reinstall";
}

function ModeBadges({ card }: { card: ModeCard }) {
  return (
    <span className="badge-row">
      {card.active && (
        <span className="pill ok">
          <Check size={11} />
          Active
        </span>
      )}
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
      {card.managed && (
        <span className="pill">
          <Cloud size={10} />
          CloudRedirect
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

  return (
    <div className="page-inner narrow">
      <PageHead
        title="Mode"
        subtitle="Pick one unlocker. They are mutually exclusive, and switching replaces the same three files."
        actions={
          <button
            className="btn ghost"
            disabled={Boolean(busy) || modes.loading}
            onClick={() => modes.reload()}
          >
            <RefreshCw size={13} />
            Check for updates
          </button>
        }
      />

      {!state?.steamPath && (
        <div className="card pad">
          <div className="section-note">
            Steam was not found. Set its folder in Settings before installing a mode.
          </div>
        </div>
      )}

      {modes.error && (
        <div className="card pad">
          <div className="section-note" style={{ color: "var(--warn)" }}>
            {modes.error.message}
          </div>
        </div>
      )}

      <div className="mode-list">
        {(state?.cards ?? []).map((card) => {
          const label = actionLabel(card);
          const working = busy === card.id;
          return (
            <article className="mode-card" key={card.id}>
              <div className="mode-body">
                <div className="mode-title">
                  <Wrench size={16} />
                  <span>{card.title}</span>
                  <ModeBadges card={card} />
                </div>
                <p className="mode-desc">{card.description}</p>
                <p className="mode-status">
                  {working ? "Working…" : card.status}
                  {card.active && card.version ? ` · ${card.version}` : ""}
                </p>
              </div>
              {label && (
                <div className="mode-action">
                  <button
                    className={`btn ${card.updateAvailable || !card.installed ? "primary" : ""}`}
                    disabled={Boolean(busy)}
                    onClick={() => {
                      if (!card.managed) void install(card);
                      else setConfirm(card);
                    }}
                  >
                    {working && <span className="spinner" />}
                    {label}
                  </button>
                </div>
              )}
            </article>
          );
        })}
      </div>

      {Boolean(busy) && <div className="progress-line" />}

      <section className={`card pad cloud-card ${state?.cloud.unlocked ? "" : "locked"}`}>
        <div className="cloud-grid">
          <div>
            <div className="mode-title">
              <Cloud size={16} style={{ color: "var(--info)" }} />
              <span>CloudRedirect</span>
            </div>
            <p className="mode-desc">
              Redirects Steam's cloud saves to a local folder so a restored build does not overwrite them.
            </p>
            <p className="mode-status">{state?.cloud.status ?? "Checking…"}</p>
          </div>
          <div className="cloud-actions">
            <button
              className="btn"
              disabled={Boolean(busy) || !state?.cloud.unlocked}
              onClick={() => void cloud(!state?.cloud.enabled, false)}
            >
              {state?.cloud.enabled ? "Disable" : "Enable"}
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
        </div>
      </section>

      <section className="card pad">
        <SectionHead title="What Steamware writes" />
        <div className="stack-tight">
          <div className="row" style={{ paddingLeft: 0 }}>
            <ShieldCheck size={15} style={{ color: "var(--ok)" }} />
            <div className="grow">
              <div className="title">Three files, nothing else</div>
              <div className="meta">
                dwmapi.dll, xinput1_4.dll and OpenSteamTool.dll in the Steam root, plus a lua path entry.
              </div>
            </div>
          </div>
          <div className="row" style={{ paddingLeft: 0 }}>
            <ArrowRight size={15} />
            <div className="grow">
              <div className="title">Verified before it lands</div>
              <div className="meta">
                The archive is checked against the release's published checksum; a mismatch writes nothing.
              </div>
            </div>
          </div>
        </div>
      </section>

      {confirm && (
        <div className="scrim" onClick={() => setConfirm(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <h2>
              {confirm.active ? "Reinstall" : "Switch to"} {confirm.title}?
            </h2>
            <p>
              Steam will close while the files are replaced, then reopen on its own. Your library and saves
              are not touched.
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
