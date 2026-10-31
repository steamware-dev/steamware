/*
 * The verified session, in the store page.
 *
 * Steamware's keyed server work — manifests, fixes, registry assets — sits behind one Turnstile check that
 * lasts an hour. (Store browsing is public and rate-limited, which is why this gate only guards the
 * add/fix paths.) The app holds that session; the plugin has none of its own, which is deliberate: a user who
 * verified in the window must not be asked again inside Steam, and a check made from Steam unlocks the
 * window too. Both read the same token, so there is exactly one session per installation.
 *
 * This script is the gate in front of "Add via Steamware". Without it the click went straight to the app,
 * which then discovered the session had lapsed and opened a browser *while* the popup claimed to be adding a
 * game — the failure looked like "it says it added it, but it didn't". Now the click asks first, and a
 * missing session opens the same verification page the app's own Verify button does.
 *
 * It runs in the capture phase so it can stop the click before the bundle's own bubble handler starts an add
 * it cannot finish.
 *
 * The notice wears the same palette as the menu skin (`window.__SteamwareSkin`, written by the injector just
 * before this file), so the toast that asks for verification matches the theme in Settings rather than
 * staying pink while the menu around it changed.
 */
(function () {
  var FALLBACK = {
    id: 'default',
    surface: '#141519',
    line: 'rgba(255, 255, 255, 0.09)',
    text: '#f4f5f8',
    textDim: '#aeb3c2',
    gradient: 'linear-gradient(135deg, #d63b96 0%, #6b2bd9 100%)',
  };
  function pick(key) {
    var live = window.__SteamwareSkin;
    var value = live && typeof live[key] === 'string' ? live[key] : '';
    return value || FALLBACK[key];
  }
  var SKIN_ID = pick('id');
  // Same single-owner arrangement as the menu skin: the gate's click handler, session refresh and toast are
  // per page, so a repaint must replace the colours without registering any of them a second time.
  var owner = window.__SteamwareSessionGate;
  var ours = !!owner && typeof owner === 'object' && typeof owner.update === 'function';
  if (ours && owner.id === SKIN_ID) return;

  /** How long a check may take before the gate gives up and says so. */
  var VERIFY_TIMEOUT = 180000;
  var POLL = 2000;
  /** Re-read the session this often while idle, so a click is answered from memory. */
  var IDLE_REFRESH = 30000;
  // The sheet and the notice carry this build's own ids, claimed by an attribute, for the same reason the
  // menu skin does: a page still being driven by a skin from an earlier build would otherwise overwrite them
  // and the notice would alternate between the brand pink and the accent from Settings.
  var STYLE = 'steamware-skin-gate';
  var MARK = 'data-sw-skin';
  var FOREIGN =
    'style[id^="steamware-gate-style"],style[id^="steamware-skin-gate"]:not([data-sw-skin]),' +
    '#steamware-gate:not([data-sw-skin])';

  var verified = false;
  /** Set while a gated click is replayed, so the gate does not catch its own second click. */
  var bypass = false;
  var pending = null;
  var toastEl = null;
  var toastTimer = null;

  function call(method, args) {
    var m = window.Millennium;
    if (!m || typeof m.callServerMethod !== "function") return Promise.reject(new Error("no bridge"));
    return m
      .callServerMethod("luatools", method, args || {})
      .then(function (result) {
        if (typeof result === "string") {
          try {
            return JSON.parse(result);
          } catch (e) {
            return null;
          }
        }
        return result;
      })
      .catch(function () {
        return null;
      });
  }

  function refresh() {
    return call("GetSteamwareSession", {}).then(function (result) {
      verified = !!(result && result.verified === true);
      return verified;
    });
  }

  function appId() {
    var match = /\/(?:app|sub)\/(\d+)/.exec(location.pathname);
    return match ? match[1] : null;
  }


  /** The store page's Add button is an Add/Remove toggle; only the Add direction is gated. */
  function isRemoval(button) {
    var mode = (button.getAttribute("data-lt-mode") || "").toLowerCase();
    if (mode === "remove") return true;
    var label = (button.textContent || "").trim().toLowerCase();
    return label.indexOf("remove") >= 0;
  }

  /* ── the notice ─────────────────────────────────────────────────────────────────────────────── */

  // Built from the palette each time it is needed, so a repaint can simply replace the stylesheet's text.
  var CSS =
    "#steamware-gate{position:fixed;right:22px;bottom:22px;z-index:2147483000;width:330px;" +
    "box-sizing:border-box;padding:15px 16px 14px;border-radius:13px;background:" + pick('surface') + ";" +
    "border:1px solid " + pick('line') + ";color:" + pick('text') + ";" +
    "font:13px/1.5 'Segoe UI',system-ui,sans-serif;" +
    "box-shadow:0 18px 44px rgba(0,0,0,.62);opacity:0;transform:translateY(8px);" +
    "transition:opacity .18s ease,transform .18s ease;pointer-events:none;}" +
    "#steamware-gate.show{opacity:1;transform:none;}" +
    "#steamware-gate .swg-head{display:flex;align-items:center;gap:9px;margin-bottom:7px;" +
    "font-weight:600;font-size:13px;letter-spacing:.1px;}" +
    "#steamware-gate .swg-dot{width:9px;height:9px;border-radius:50%;flex:none;" +
    "background:" + pick('gradient') + ";box-shadow:0 0 10px " + pick('glowSoft') + ";}" +
    "#steamware-gate.ok .swg-dot{background:#5ad1a0;box-shadow:0 0 10px rgba(90,209,160,.7);}" +
    "#steamware-gate.err .swg-dot{background:#e5484d;box-shadow:0 0 10px rgba(229,72,77,.7);}" +
    "#steamware-gate .swg-body{color:" + pick('textDim') + ";font-size:12.5px;line-height:1.55;}" +
    "#steamware-gate .swg-bar{height:2px;margin-top:11px;border-radius:2px;overflow:hidden;" +
    "background:" + pick('hover') + ";}" +
    "#steamware-gate .swg-bar i{display:block;height:100%;width:38%;border-radius:2px;" +
    "background:" + pick('gradient') + ";animation:swg-slide 1.25s ease-in-out infinite;}" +
    "@keyframes swg-slide{0%{transform:translateX(-100%)}100%{transform:translateX(320%)}}";

  function currentCss() {
    var live = window.__SteamwareSessionGate;
    return live && typeof live.css === 'string' ? live.css : CSS;
  }

  // A repaint under an owner this build installed: swap the colours and keep the one click handler.
  if (ours) {
    owner.update(CSS, SKIN_ID);
    return;
  }

  window.__SteamwareSessionGate = {
    id: SKIN_ID,
    css: CSS,
    update: function (css, id) {
      var live = window.__SteamwareSessionGate;
      if (!live || typeof live !== 'object') return;
      live.css = css;
      live.id = id;
      window.__SteamwareSessionGateId = id;
      ensureStyle();
    },
  };
  window.__SteamwareSessionGateId = SKIN_ID;

  /** One palette per page: any sheet or notice a previous build left behind is removed, ours is claimed. */
  function sweepForeign() {
    try {
      var foreign = document.querySelectorAll(FOREIGN);
      for (var i = 0; i < foreign.length; i++) {
        if (foreign[i].parentNode) foreign[i].parentNode.removeChild(foreign[i]);
      }
    } catch (e) {}
  }

  function ensureStyle() {
    sweepForeign();
    var el = document.getElementById(STYLE);
    if (!el) {
      el = document.createElement("style");
      el.id = STYLE;
      (document.head || document.documentElement).appendChild(el);
    }
    if (!el.getAttribute(MARK)) el.setAttribute(MARK, '1');
    var css = currentCss();
    if (el.textContent !== css) el.textContent = css;
    if (!toastEl) {
      toastEl = document.createElement("div");
      toastEl.id = "steamware-gate";
      toastEl.setAttribute(MARK, '1');
      toastEl.setAttribute("role", "status");
      document.body.appendChild(toastEl);
    }
  }

  /**
   * @param {string} title
   * @param {string} body
   * @param {"busy"|"ok"|"err"} kind
   * @param {boolean} sticky
   */
  function notice(title, body, kind, sticky) {
    try {
      ensureStyle();
      toastEl.className = "show" + (kind === "ok" ? " ok" : kind === "err" ? " err" : "");
      var bar = kind === "busy" ? '<div class="swg-bar"><i></i></div>' : "";
      toastEl.innerHTML =
        '<div class="swg-head"><span class="swg-dot"></span><span></span></div>' +
        '<div class="swg-body"></div>' +
        bar;
      toastEl.querySelector(".swg-head span:last-child").textContent = title;
      toastEl.querySelector(".swg-body").textContent = body;
      if (toastTimer) {
        clearTimeout(toastTimer);
        toastTimer = null;
      }
      if (!sticky) {
        toastTimer = setTimeout(function () {
          if (toastEl) toastEl.className = "";
        }, kind === "err" ? 9000 : 4500);
      }
    } catch (e) {}
  }

  /* ── the gate ──────────────────────────────────────────────────────────────────────────────── */

  function waitForSession() {
    var started = Date.now();
    return new Promise(function (resolve) {
      (function poll() {
        refresh().then(function (ok) {
          if (ok) return resolve(true);
          if (Date.now() - started > VERIFY_TIMEOUT) return resolve(false);
          setTimeout(poll, POLL);
        });
      })();
    });
  }

  function gate(button) {
    if (pending) return;
    pending = { button: button, appid: appId() };
    notice(
      "Verify to add games",
      "A quick check in your browser unlocks Steamware and this plugin — once for both.",
      "busy",
      true,
    );

    call("VerifySteamwareSession", {}).then(function (result) {
      if (!result) {
        pending = null;
        notice(
          "Steamware isn't running",
          "Open the Steamware app, then click Add via Steamware again.",
          "err",
          false,
        );
        return;
      }
      waitForSession().then(function (ok) {
        var target = pending;
        pending = null;
        if (!ok) {
          notice("Verification didn't finish", "Try again, or verify from the Steamware window.", "err", false);
          return;
        }
        notice("Verified", "Steamware is unlocked for the next hour.", "ok", false);
        if (!target || !target.button || !target.button.isConnected) return;
        // Replay the click the gate swallowed. `bypass` keeps this from re-entering the gate; the bundle's
        // own handler then runs its add exactly as if the user had clicked with a live session.
        bypass = true;
        try {
          target.button.click();
        } finally {
          setTimeout(function () {
            bypass = false;
          }, 0);
        }
      });
    });
  }

  function onClick(event) {
    if (bypass) return;
    var target = event.target;
    if (!target || !target.closest) return;
    var button = target.closest("a.luatools-button, .luatools-button");
    if (!button || isRemoval(button)) return;
    // A verified session needs no ceremony: let the click through untouched.
    if (verified) return;
    // The app id is only for the log line: the gate itself applies to the button, so a page whose URL shape
    // this does not recognise is still gated rather than silently let through.
    event.preventDefault();
    event.stopPropagation();
    if (event.stopImmediatePropagation) event.stopImmediatePropagation();
    gate(button);
  }

  // Capture, so the gate sees the click before the bundle's bubble-phase handler starts an add.
  document.addEventListener("click", onClick, true);

  function start() {
    ensureStyle();
    refresh();
    setInterval(function () {
      if (!pending) refresh();
    }, IDLE_REFRESH);
    // Keeps the one-palette rule true over time on a page an older build is still repainting.
    try {
      setInterval(ensureStyle, 5000);
    } catch (e) {}
    // A check made in the app window should unblock the next click here without waiting out the interval.
    try {
      window.addEventListener("focus", function () {
        if (!pending) refresh();
      });
    } catch (e) {}
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", start);
  else start();
})();
