/*
 * The Steamware look for the in-Steam plugin's own chrome.
 *
 * The bundled plugin draws its settings menu and alerts from its theme palette, which is Steam's blue, and it
 * does so with *inline* styles on a freshly built panel every time the menu opens: a blue gradient, a
 * `rgb(102,192,244)` border and a blue glow. `!important` rules are the only way to outrank that, so this
 * overlay carries literal colours and no CSS variables.
 *
 * Those colours are the app's own. `window.__SteamwareSkin` is written by the injector immediately before
 * this file runs (see `skin.rs`), and holds the palette derived from the accent and light/dark mode chosen in
 * Settings — the gradient starts on that exact accent. A page painted before an accent change announces the
 * old signature in `window.__SteamwareSkinId`, which is what makes the injector re-run this script; running
 * again with a new palette only rebuilds the stylesheet, it does not add a second observer.
 *
 * Two details keep it from ever losing:
 *
 * 1. The stylesheet is moved to the end of `<head>` on every pass. The plugin rewrites its own
 *    `#luatools-styles` at runtime, and a later stylesheet wins any tie; sitting last means ties go to us.
 * 2. A MutationObserver re-asserts the stylesheet the moment the menu (or its overlay) is inserted, so the
 *    panel never paints blue first and then corrects itself a poll later. The interval is only the fallback
 *    for a page that replaces `<head>` wholesale.
 * 3. Only one skin may exist on the page. The sheet id here is one no earlier build ever wrote
 *    (`steamware-menu-skin` was the old name) and it is claimed by an attribute, so a second injector — a
 *    page from before an update that is still being driven by its own interval — cannot write into our
 *    element. Such a sheet is deleted on sight instead; when both wrote the same id, each pass overwrote
 *    the other and the panel visibly alternated between the brand pink and the accent from Settings.
 */
(function () {
  // The plugin's original pair, kept as the fallback so a page where the palette never arrived (an older
  // injection, a hand-edited bundle) still gets the brand rather than nothing.
  var FALLBACK = {
    id: 'default',
    surface: '#141519',
    raised: '#1b1d23',
    line: 'rgba(255, 255, 255, 0.09)',
    text: '#f4f5f8',
    textDim: '#aeb3c2',
    hover: 'rgba(255, 255, 255, 0.055)',
    gradient: 'linear-gradient(135deg, #d63b96 0%, #6b2bd9 100%)',
    gradientHot: 'linear-gradient(135deg, #ec4fae 0%, #7d43ef 100%)',
    glow: 'rgba(107, 43, 217, 0.42)',
    glowSoft: 'rgba(214, 59, 150, 0.50)',
    icon: '#c9a8f5',
  };

  function pick(key) {
    var live = window.__SteamwareSkin;
    var value = live && typeof live[key] === 'string' ? live[key] : '';
    return value || FALLBACK[key];
  }

  var SKIN_ID = pick('id');
  // One owner per page, holding the palette it was last asked to paint. A page injected by an older build
  // (or by a run of this file from before the palette existed) has `true` here instead, which is why the
  // owner is an object and not a flag: a repaint must replace the colours, never add a second observer that
  // would fight the first one over which palette is current.
  var owner = window.__SteamwareMenuSkin;
  var ours = !!owner && typeof owner === 'object' && typeof owner.update === 'function';
  if (ours && owner.id === SKIN_ID) return;

  var STYLE = 'steamware-skin-sheet';
  var MARK = 'data-sw-skin';
  // Every sheet a skin from any earlier build can leave here: its own id, and the marker-less twin of ours.
  var FOREIGN = 'style[id^="steamware-menu-skin"],style[id^="steamware-skin-sheet"]:not([data-sw-skin])';
  var SURFACE = pick('surface');
  var RAISED = pick('raised');
  var LINE = pick('line');
  var TEXT = pick('text');
  var TEXT_DIM = pick('textDim');
  var HOVER = pick('hover');
  var GRADIENT = pick('gradient');
  var GRADIENT_HOT = pick('gradientHot');
  var GLOW = pick('glow');
  var GLOW_SOFT = pick('glowSoft');
  var ICON = pick('icon');

  var CSS = `
.luatools-settings-overlay,.luatools-overlay,.luatools-loadedapps-overlay{background:rgba(0,0,0,.6)!important;backdrop-filter:blur(6px)!important;-webkit-backdrop-filter:blur(6px)!important;}
/* Structural selectors, not classes added afterwards: the overlay is appended complete in one go, so the
   panel is styled on its first paint and opening the menu never flashes the plugin's blue design. The
   plugin owns this markup (its ids are the only stable hooks), hence the child paths. */
.luatools-settings-overlay>div{width:520px!important;max-width:calc(100vw - 32px)!important;padding:0!important;border-radius:16px!important;background:${SURFACE}!important;background-image:none!important;border:1px solid ${LINE}!important;box-shadow:0 24px 60px rgba(0,0,0,.62)!important;overflow:hidden!important;color:${TEXT}!important;animation:none!important;}
.luatools-settings-overlay>div>div:first-child{padding:16px 14px 14px 20px!important;margin:0!important;border-bottom:1px solid ${LINE}!important;}
.luatools-settings-overlay>div>div:first-child>div:first-child{font-size:15px!important;font-weight:600!important;letter-spacing:.2px!important;gap:10px!important;color:${TEXT}!important;}
.luatools-settings-overlay>div>div:first-child>div:first-child img{width:24px!important;height:24px!important;border-radius:6px!important;}
.luatools-settings-overlay>div>div:first-child>div:last-child{gap:4px!important;}
.luatools-settings-overlay>div>div:first-child>div:last-child a{width:34px!important;height:34px!important;border-radius:9px!important;background:transparent!important;border:1px solid transparent!important;color:${TEXT_DIM}!important;font-size:14px!important;box-shadow:none!important;transform:none!important;transition:background .15s ease,color .15s ease,box-shadow .15s ease!important;}
/* Discord and settings share the brand gradient with the panel's actions, so nothing in the menu is blue. */
.luatools-settings-overlay>div>div:first-child>div:last-child a:not(#lt-settings-close):hover{background:${GRADIENT}!important;color:#fff!important;border-color:transparent!important;box-shadow:0 8px 20px ${GLOW}!important;transform:none!important;}
#lt-settings-close:hover{background:${HOVER}!important;color:${TEXT}!important;transform:none!important;}
.luatools-settings-overlay>div>div:last-child{padding:16px 20px 18px!important;margin:0!important;font-size:13px!important;line-height:1.5!important;}
.luatools-settings-overlay>div>div:last-child>div{margin-top:0!important;gap:10px!important;}
.luatools-settings-overlay>div>div:last-child>div:last-child{display:flex!important;gap:10px!important;justify-content:center!important;}
#lt-settings-remove-lua{justify-content:flex-start!important;padding:10px 14px!important;border-radius:10px!important;background:transparent!important;border:1px solid ${LINE}!important;color:${TEXT_DIM}!important;font-size:13px!important;font-weight:500!important;box-shadow:none!important;transform:none!important;transition:background .15s ease,border-color .15s ease,color .15s ease!important;}
#lt-settings-remove-lua:hover{background:rgba(229,72,77,.12)!important;border-color:rgba(229,72,77,.45)!important;color:#fff!important;}
#lt-settings-fixes-menu,#lt-settings-check{flex-direction:row!important;justify-content:flex-start!important;gap:11px!important;padding:13px 15px!important;border-radius:10px!important;background:transparent!important;border:1px solid ${LINE}!important;color:${TEXT}!important;font-size:13px!important;font-weight:600!important;box-shadow:none!important;transform:none!important;transition:background .15s ease,border-color .15s ease,box-shadow .15s ease,color .15s ease!important;}
#lt-settings-fixes-menu:hover,#lt-settings-check:hover{background:${GRADIENT}!important;border-color:transparent!important;color:#fff!important;box-shadow:0 10px 26px ${GLOW}!important;transform:none!important;}
#lt-settings-fixes-menu:hover i,#lt-settings-check:hover i{color:#fff!important;}
#lt-settings-fixes-menu i,#lt-settings-check i{width:20px!important;font-size:16px!important;text-align:center!important;color:${ICON}!important;transition:color .15s ease!important;}
#lt-settings-fixes-menu span,#lt-settings-check span{text-align:left!important;line-height:1.3!important;}
/* Alerts are their own overlay in the bundle; give them the same surface so a result from the menu does not
   land in a separate blue dialog. */
.luatools-alert-overlay{background:rgba(0,0,0,.6)!important;backdrop-filter:blur(6px)!important;-webkit-backdrop-filter:blur(6px)!important;}
.luatools-alert-overlay>div{width:440px!important;max-width:calc(100vw - 32px)!important;padding:20px 22px 18px!important;border-radius:16px!important;background:${SURFACE}!important;background-image:none!important;border:1px solid ${LINE}!important;box-shadow:0 24px 56px rgba(0,0,0,.6)!important;color:${TEXT}!important;}
/* The alert's icon, title and message each carry their own inline theme colour, so each one is named. Without
   this a light plugin theme would print dark text on the panel. */
.luatools-alert-overlay>div>div:first-child{color:${ICON}!important;}
.luatools-alert-overlay>div>div:first-child i{color:${ICON}!important;}
.luatools-alert-overlay>div>div:nth-child(2){color:${TEXT}!important;font-size:17px!important;}
.luatools-alert-overlay>div>div:nth-child(3){color:${TEXT_DIM}!important;}
.luatools-alert-overlay>div>div:first-child img{border-radius:7px!important;}
.luatools-alert-overlay .luatools-btn.primary{background:${GRADIENT}!important;border-color:transparent!important;color:#fff!important;box-shadow:0 10px 24px ${GLOW}!important;}
.luatools-alert-overlay .luatools-btn.primary span{color:#fff!important;}
.luatools-alert-overlay .luatools-btn.primary:hover{background:${GRADIENT_HOT}!important;}
/* The add-game picker ("Select Download Source") is a *third* overlay, and it is the one that opens from a
   game's store page. It carries the plugin's theme palette inline all the way down — the panel, every
   source row, the progress bar and its button — so without this block a game page showed the old blue
   dialog while the shop page showed the black one. Child paths are used because the markup is built at
   runtime with no classes on the parts that matter. */
.luatools-overlay>div{width:520px!important;max-width:calc(100vw - 32px)!important;padding:22px 24px 18px!important;border-radius:16px!important;background:${SURFACE}!important;background-image:none!important;border:1px solid ${LINE}!important;box-shadow:0 24px 60px rgba(0,0,0,.62)!important;color:${TEXT}!important;animation:none!important;}
.luatools-overlay>div>div:first-child{display:flex!important;align-items:center!important;gap:10px!important;margin-bottom:14px!important;font-size:16px!important;font-weight:600!important;color:${TEXT}!important;}
.luatools-overlay>div>div:first-child i{color:${ICON}!important;font-size:17px!important;}
.luatools-overlay .luatools-api-item{background:${RAISED}!important;border:1px solid ${LINE}!important;border-radius:10px!important;padding:11px 14px!important;margin-bottom:8px!important;box-shadow:none!important;transition:background .15s ease,border-color .15s ease!important;}
.luatools-overlay .luatools-api-item:hover{background:${HOVER}!important;border-color:${GLOW_SOFT}!important;}
.luatools-overlay .luatools-api-item>div:first-child{color:${TEXT}!important;font-size:13px!important;font-weight:600!important;}
.luatools-overlay .luatools-status{color:${TEXT_DIM}!important;font-size:13px!important;}
.luatools-overlay .luatools-status i{color:${ICON}!important;}
.luatools-overlay .luatools-progress-wrap{height:8px!important;border-radius:999px!important;background:${RAISED}!important;border:1px solid ${LINE}!important;overflow:hidden!important;}
.luatools-overlay .luatools-progress-bar{background:${GRADIENT}!important;box-shadow:none!important;}
.luatools-overlay .luatools-progress-info,.luatools-overlay .luatools-percent{color:${TEXT_DIM}!important;}
.luatools-overlay .luatools-btn{padding:10px 20px!important;border-radius:10px!important;background:transparent!important;border:1px solid ${LINE}!important;color:${TEXT}!important;box-shadow:none!important;transform:none!important;transition:background .15s ease,border-color .15s ease,color .15s ease!important;}
.luatools-overlay .luatools-btn:hover{background:${GRADIENT}!important;border-color:transparent!important;color:#fff!important;}
.luatools-overlay .luatools-btn span{color:inherit!important;}
/* Confirming an add uses this overlay; same surface, same two buttons, so it cannot fall back to blue. */
.luatools-confirm-overlay>div{width:420px!important;max-width:calc(100vw - 32px)!important;padding:26px 28px 22px!important;border-radius:16px!important;background:${SURFACE}!important;background-image:none!important;border:1px solid ${LINE}!important;box-shadow:0 24px 60px rgba(0,0,0,.62)!important;color:${TEXT}!important;animation:none!important;}
.luatools-confirm-overlay>div>div:nth-child(1) i{color:${ICON}!important;}
.luatools-confirm-overlay>div>div:nth-child(2){color:${TEXT}!important;font-size:19px!important;font-weight:600!important;}
.luatools-confirm-overlay>div>div:nth-child(3){color:${TEXT_DIM}!important;}
.luatools-confirm-overlay .luatools-btn{flex:1!important;padding:11px 16px!important;border-radius:10px!important;background:transparent!important;border:1px solid ${LINE}!important;color:${TEXT}!important;box-shadow:none!important;transform:none!important;}
.luatools-confirm-overlay .luatools-btn:hover{background:${HOVER}!important;}
.luatools-confirm-overlay .luatools-btn.primary{background:${GRADIENT}!important;border-color:transparent!important;color:#fff!important;box-shadow:0 10px 26px ${GLOW}!important;}
.luatools-confirm-overlay .luatools-btn.primary:hover{background:${GRADIENT_HOT}!important;}
.luatools-confirm-overlay .luatools-btn span{color:inherit!important;}
/* The add-on's own list of managed apps shares the surface, so opening it from the menu is not a jump back
   to the plugin's palette. */
.luatools-loadedapps-overlay>div{background:${SURFACE}!important;background-image:none!important;border:1px solid ${LINE}!important;border-radius:16px!important;box-shadow:0 24px 60px rgba(0,0,0,.62)!important;color:${TEXT}!important;animation:none!important;}
.luatools-loadedapps-overlay .luatools-btn{padding:10px 18px!important;border-radius:10px!important;background:transparent!important;border:1px solid ${LINE}!important;color:${TEXT}!important;box-shadow:none!important;}
.luatools-loadedapps-overlay .luatools-btn:hover{background:${HOVER}!important;}
/* The panel's own controls are painted from the plugin's palette too — blue selects on this card read as
   two different programs. */
.luatools-settings-overlay select,.luatools-settings-overlay input[type="text"],.luatools-settings-overlay input[type="password"],.luatools-settings-overlay textarea{background:${RAISED}!important;border:1px solid ${LINE}!important;color:${TEXT}!important;box-shadow:none!important;}
.luatools-settings-overlay select option{background:${RAISED}!important;color:${TEXT}!important;}
`;

  // The owner's copy, not this closure's: a repaint swaps the text and every pass of every instance then
  // paints the same colours, which is what stops two runs from overwriting each other in a loop.
  function currentCss() {
    var live = window.__SteamwareMenuSkin;
    return live && typeof live.css === 'string' ? live.css : CSS;
  }

  /** Delete any sheet another Steamware left on this page, so a single palette is the only one that can
   * paint. Runs before ours is placed: a foreign sheet added by an older injection's timer is gone in the
   * same pass, so the panel never shows its colours even for one frame.
   *
   * Only asked to run when it can have something to do — once at the start, whenever a Steamware sheet is
   * inserted, and once a second — because a store page mutates hundreds of times a second and this pass is
   * on the same path the menu's own insertion uses. */
  function sweepForeign() {
    try {
      var foreign = document.querySelectorAll(FOREIGN);
      for (var i = 0; i < foreign.length; i++) {
        if (foreign[i].parentNode) foreign[i].parentNode.removeChild(foreign[i]);
      }
    } catch (e) {}
  }

  function ensureStyle() {
    var head = document.head || document.documentElement;
    if (sweepWanted) {
      sweepWanted = false;
      sweepForeign();
    }
    var el = document.getElementById(STYLE);
    if (!el) {
      el = document.createElement('style');
      el.id = STYLE;
    }
    if (!el.getAttribute(MARK)) el.setAttribute(MARK, '1');
    var css = currentCss();
    if (el.textContent !== css) el.textContent = css;
    // Re-appending an already-last element is a no-op for the DOM, but it is what makes this sheet win ties
    // against the plugin's own `#luatools-styles`, which it rewrites in place — and against the sheets of an
    // injection from an earlier app build, which Steam can keep running until the page is reloaded.
    if (head.lastElementChild !== el) head.appendChild(el);
  }

  // Set at the start and by anything that inserts a Steamware sheet: the only two ways a second skin can
  // appear on the page.
  var sweepWanted = true;

  var pending = false;
  function schedule() {
    if (pending) return;
    pending = true;
    // Microtask, so the menu is repainted in the same frame it was inserted rather than on the next poll.
    Promise.resolve().then(function () {
      pending = false;
      ensureStyle();
    });
  }

  function start() {
    ensureStyle();
    try {
      var observer = new MutationObserver(function (records) {
        for (var i = 0; i < records.length; i++) {
          var nodes = records[i].addedNodes;
          for (var j = 0; j < nodes.length; j++) {
            var node = nodes[j];
            if (!node || node.nodeType !== 1) continue;
            var cls = String(node.className || '');
            if (cls.indexOf('luatools-settings-overlay') >= 0 || cls.indexOf('luatools-alert-overlay') >= 0 ||
                cls.indexOf('luatools-overlay') >= 0 || cls.indexOf('luatools-loadedapps-overlay') >= 0) {
              schedule();
              return;
            }
            if (node.id === STYLE) return;
            // The plugin re-creating its own stylesheet (or any other) would put a later sheet ahead of
            // ours, which is exactly the tie a blue panel wins. Reclaim last place when that happens.
            if (node.tagName === 'STYLE' || node.tagName === 'LINK') {
              var addedId = String(node.id || '');
              if (addedId.indexOf('steamware-') === 0) sweepWanted = true;
              schedule();
            }
          }
        }
        // A removed sheet (or a replaced <head>) is the other way the skin goes missing.
        if (!document.getElementById(STYLE)) schedule();
      });
      observer.observe(document.documentElement, { childList: true, subtree: true });
    } catch (e) {}
    // Fallback for a page that swaps <head> wholesale: nothing observable above would notice. Once a second,
    // so that a stale sheet re-appended on a timer by an older injection is taken back promptly.
    try {
      setInterval(function () {
        sweepWanted = true;
        ensureStyle();
      }, 1000);
    } catch (e) {}
  }

  // A repaint under an owner this build installed: swap the colours and keep the one observer.
  if (ours) {
    owner.update(CSS, SKIN_ID);
    return;
  }

  window.__SteamwareMenuSkin = {
    id: SKIN_ID,
    css: CSS,
    update: function (css, id) {
      var live = window.__SteamwareMenuSkin;
      if (!live || typeof live !== 'object') return;
      live.css = css;
      live.id = id;
      window.__SteamwareMenuSkinId = id;
      ensureStyle();
    },
  };
  window.__SteamwareMenuSkinId = SKIN_ID;

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
