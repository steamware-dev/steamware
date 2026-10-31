/*
 * A home for the plugin's settings icon in Steam's current store header.
 *
 * The bundle builds its cogwheel only when it finds `._1wn1lBlAzl3HMRqS1llwie` — the React hash class of the
 * store header's action row. Steam re-hashes those classes with every client update, and in this build the
 * class is gone entirely: the header is now a plain nav row (Browse / Recommendations / … / search /
 * Wishlist) rendered by the store page itself.
 *
 * That one selector was doing two jobs, so its disappearance broke two things at once. The icon was never
 * created, so there was no visible way into the plugin's settings; and because the bundle marks itself ready
 * only once that icon exists, the injector judged every page un-injected and re-evaluated the whole ~550 KB
 * bundle once a second, forever. Both are fixed by giving the bundle the container it asks for.
 *
 * So this does not draw a button of its own: it inserts an empty element carrying the old class into the
 * modern row, and the bundle fills it with its real icon (tooltip, settings popup and all). If a later Steam
 * build brings the class back, the first lookup matches and this script does nothing.
 */
(function () {
  var HOST_CLASS = '_1wn1lBlAzl3HMRqS1llwie';
  var OWN_MARK = 'data-sw-plugin-header';

  // The bundle already has somewhere to put its icon — either Steam still ships the class, or a previous
  // pass of this script made the host and the bundle has since filled it.
  function settled() {
    return !!document.querySelector('button.luatools-header-button') || !!document.querySelector('.' + HOST_CLASS);
  }

  /**
   * The search box in the store's top nav. Anchored on the input rather than on the "Wishlist" label
   * because the store is localised and that label is translated on every non-English client.
   */
  function navSearch() {
    var inputs = document.querySelectorAll('input');
    for (var i = 0; i < inputs.length; i++) {
      var input = inputs[i];
      if (input.type !== 'text' && input.type !== 'search') continue;
      var rect = input.getBoundingClientRect();
      // Top bar only: the page's own content search and the hidden inputs elsewhere must not match.
      if (rect.width < 100 || rect.height < 16 || rect.top < 0 || rect.top > 120) continue;
      return input;
    }
    return null;
  }

  function makeHost() {
    var host = document.createElement('div');
    host.className = HOST_CLASS;
    host.setAttribute(OWN_MARK, '1');
    // The bundle styles the button it puts here (36px square with a 12px left margin), so the host only has
    // to take the row's full height and centre it. A fixed shorter height left the icon one pixel proud of
    // the bar, which the header's own overflow then clipped.
    host.style.cssText =
      'display:flex;align-items:center;align-self:stretch;margin-left:8px;flex:0 0 auto;';
    return host;
  }

  function place() {
    if (settled()) return true;

    var search = navSearch();
    if (search) {
      // [search box] → [its wrapper] → [the row that also holds Wishlist]. Inserting straight after the
      // search wrapper keeps the icon on the right of the bar without depending on any class name.
      var box = search.parentElement;
      var outer = box && box.parentElement;
      var row = (outer && outer.parentElement) || outer;
      if (row) {
        row.insertBefore(makeHost(), outer.nextSibling);
        return true;
      }
    }

    // Nothing yet. Deliberately no second location: the nav is present on the store's front page and on
    // every app page, and a fallback that placed the icon in the app's own header block made it appear in
    // two different spots depending on how far the page had loaded when the injector ran. The observer and
    // the retries below try again until the nav is there.
    return false;
  }

  var placed = false;
  try {
    placed = place();
  } catch (e) {}

  // React re-renders the nav (and drops unknown children with it) on SPA navigation and on some focus
  // changes. Re-assert the host only when it is genuinely missing, so the observer converges instead of
  // fighting the page. The bundle's icon is left alone: if it is there, there is nothing to repair.
  try {
    var pending = false;
    var observer = new MutationObserver(function () {
      if (pending) return;
      if (document.querySelector('button.luatools-header-button')) return;
      pending = true;
      setTimeout(function () {
        pending = false;
        try {
          place();
        } catch (e) {}
      }, 250);
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
  } catch (e) {}

  // A store page that renders its nav after the bundle gave up retrying still gets the icon on a later
  // pass of this script; a handful of spaced attempts covers the load without a polling loop.
  if (!placed) {
    try {
      setTimeout(place, 600);
      setTimeout(place, 1800);
      setTimeout(place, 4000);
    } catch (e) {}
  }
})();
