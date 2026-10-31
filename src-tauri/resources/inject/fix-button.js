/*
 * "Fix game" on a Steam store page.
 *
 * The plugin's own Fixes menu runs inside Steam. Steamware has a Fixes page that searches by AppID, so the
 * store page gets a single button that opens the app already filtered to this game — one click from "this
 * game has a fix" to the fix itself.
 *
 * It no longer hangs off the Add button alone: `a.luatools-button` is only inserted when the game is NOT
 * already in the library, which meant a game you already own showed no Fix button at all. The row's other
 * permanent anchor is the plugin's Restart button, so the order tried is Add → Restart → the row itself.
 */
(function () {
  if (window.__SteamwareFix) return;
  window.__SteamwareFix = true;

  var CLS = 'steamware-fix-button';
  var STYLE = 'steamware-fix-style';
  var state = { appid: null, has: false, inFlight: false };
  // The whitespace node this script seeds after the button (see insert); kept so remove() can take it back
  // out instead of leaving a stray gap in Steam's row.
  var spacer = null;

  // The button sits in Steam's own action row, so it is painted with Steam's own action colour rather than
  // the Steamware gradient: the same translucent blue fill, light-blue label and solid hover fill that the
  // Community Hub / Store Page buttons beside it use. A gradient pill in that row read as an advert bolted
  // onto the client; this reads as one more Steam button that happens to be ours.
  var BG = 'rgba(103, 193, 245, 0.2)';
  var BG_HOT = '#67c1f5';
  var INK = '#67c1f5';

  // Geometry too: 1px inset, 30px of line, a 15px label and a 15px inner gutter — measured off the Community
  // Hub button in that same row, so the three buttons are one run rather than two Steam buttons and a sticker.
  var CSS =
    'a.' + CLS + '{' +
    'display:inline-block!important;height:30px!important;padding:1px!important;' +
    'background:' + BG + '!important;background-image:none!important;color:' + INK + '!important;' +
    'border:0!important;border-radius:2px!important;font-size:15px!important;font-weight:400!important;' +
    'line-height:30px!important;letter-spacing:0!important;vertical-align:top!important;' +
    'box-shadow:none!important;text-decoration:none!important;margin:0 6px!important;' +
    'transition:background .12s ease,color .12s ease!important;}' +
    'a.' + CLS + ':hover{background:' + BG_HOT + '!important;background-image:none!important;color:#fff!important;' +
    'box-shadow:none!important;transform:none!important;}' +
    'a.' + CLS + ' span{display:inline-block!important;padding:0 15px!important;line-height:30px!important;' +
    'font-size:15px!important;font-weight:400!important;color:inherit!important;}';

  function ensureStyle() {
    var el = document.getElementById(STYLE);
    if (!el) {
      el = document.createElement('style');
      el.id = STYLE;
      (document.head || document.documentElement).appendChild(el);
    }
    if (el.textContent !== CSS) el.textContent = CSS;
  }

  function appId() {
    var m = /\/app\/(\d+)/.exec(location.pathname);
    return m ? m[1] : null;
  }

  function existing() {
    return document.querySelector('a.' + CLS);
  }

  function remove() {
    var b = existing();
    if (b && b.parentNode) b.parentNode.removeChild(b);
    if (spacer && spacer.parentNode) spacer.parentNode.removeChild(spacer);
    spacer = null;
  }

  function isSpace(node) {
    return !!node && node.nodeType === 3 && !/\S/.test(node.nodeValue || '');
  }

  /** Where the button belongs: next to Add when the game is new, otherwise next to Restart. */
  function anchor() {
    var add = document.querySelector('a.luatools-button');
    if (add && add.parentNode) return { parent: add.parentNode, after: add };
    var restart = document.querySelector('.luatools-restart-button');
    if (restart && restart.parentNode) return { parent: restart.parentNode, after: restart };
    return null;
  }

  function insert() {
    var spot = anchor();
    if (!spot) return;
    if (existing()) return;

    var el = document.createElement('a');
    el.href = '#';
    // Steam's own button classes as well as ours: they bring the client's hover fade, and the CSS above is
    // the floor that keeps the button looking right if that markup ever changes.
    el.className = CLS + ' btnv6_blue_hoverfade btn_medium';
    el.title = 'Fix game — open in Steamware';
    el.setAttribute('data-tooltip-text', 'Fix game');

    var span = document.createElement('span');
    span.textContent = 'Fix game';
    el.appendChild(span);

    el.addEventListener('click', function (e) {
      e.preventDefault();
      e.stopPropagation();
      var id = appId() || state.appid;
      if (!id) return;
      try {
        window.Millennium.callServerMethod('luatools', 'OpenFixForApp', { appid: id });
      } catch (err) {}
    });

    spot.parent.insertBefore(el, spot.after.nextSibling);

    // Steam separates its own buttons with a whitespace text node (~3px at this type size) on top of each
    // button's margin, and the 6px margin in the CSS is measured against that: together they are the ~9px
    // gap between Community Hub and the button beside it. When the fix button lands straight against the
    // next button there is no such node — which is why it looked glued to Restart — so seed the same
    // whitespace it already has on its left.
    if (!isSpace(el.nextSibling)) {
      spacer = document.createTextNode(' ');
      spot.parent.insertBefore(spacer, el.nextSibling);
    }
  }

  function check(force) {
    ensureStyle();
    var id = appId();
    if (!id) {
      remove();
      state = { appid: null, has: false, inFlight: false };
      return;
    }
    if (!force && state.appid === id) {
      if (state.has) insert();
      else remove();
      return;
    }
    if (state.inFlight) return;
    var m = window.Millennium;
    if (!m || typeof m.callServerMethod !== 'function') return;
    state.inFlight = true;
    m.callServerMethod('luatools', 'CheckFixForApp', { appid: id })
      .then(function (r) {
        state.inFlight = false;
        if (appId() !== id) return;
        var has = !!(r && r.hasFix === true);
        state = { appid: id, has: has, inFlight: false };
        if (has) insert();
        else remove();
      })
      .catch(function () {
        // Couldn't ask (offline, rate limited): leave whatever is on screen alone rather than claiming this
        // game has no fix.
        state.inFlight = false;
      });
  }

  // A busy store page mutates hundreds of times a second (carousels, wishlist buttons, price timers), and
  // every one of those would otherwise re-enter check() and force a style recalculation. One coalesced pass
  // per frame is all the button ever needs.
  var scheduled = false;
  function schedule() {
    if (scheduled) return;
    scheduled = true;
    setTimeout(function () {
      scheduled = false;
      check(false);
    }, 120);
  }

  function start() {
    check(true);
    try {
      var mo = new MutationObserver(schedule);
      mo.observe(document.documentElement, { childList: true, subtree: true });
    } catch (e) {}
    // The Add/Restart row lands after this script on a store SPA render, so a few one-shot re-checks cover
    // the gap; the interval keeps the answer fresh without re-asking on every mutation.
    try {
      setTimeout(function () {
        check(true);
      }, 1200);
      setTimeout(function () {
        check(true);
      }, 4000);
      setInterval(function () {
        check(false);
      }, 3000);
    } catch (e) {}
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start);
  else start();
})();
