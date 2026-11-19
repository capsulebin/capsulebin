/* Moves the page to the preferred interface language before anything is shown.
 *
 * Every page exists once per language under a path prefix (/en/..., /de/...).
 * The server redirects prefix-less links by Accept-Language; this script then
 * applies the order the browser knows better:
 *
 *   1. the choice stored by the language switch (localStorage, if offered),
 *   2. the first entry of navigator.languages whose primary subtag is offered,
 *   3. the operator's default language.
 *
 * If that differs from the prefix of the current path, it replaces the page with
 * the same path under the other prefix. Query and fragment are kept unchanged -
 * the fragment holds the key and never leaves the browser.
 *
 * The offered languages come from <meta name="capsulebin-languages">, which the
 * server puts into every shell. Without it (Vite dev server), without
 * localStorage, or on any exception the script does nothing.
 *
 * Like theme-init.js this is an external classic script without defer/async, so
 * that it runs before the first paint, and it runs before theme-init.js because
 * after a redirect the theme no longer matters. The storage key is duplicated
 * from src/language.ts on purpose (importing it would make this a deferred
 * module); a test keeps both in sync.
 */
(function () {
    try {
        var meta = document.querySelector('meta[name="capsulebin-languages"]');
        if (!meta) return;
        var offered = (meta.getAttribute('content') || '').split(/\s+/).filter(Boolean);
        var stored = localStorage.getItem('capsulebin.language');

        var match = /^\/([^/]+)(.*)$/.exec(location.pathname);
        if (!match || offered.indexOf(match[1]) < 0) return;
        var current = match[1];
        var rest = match[2];

        var lang = null;
        if (stored && offered.indexOf(stored) >= 0) {
            lang = stored;
        }
        var preferred = navigator.languages || [];
        for (var i = 0; !lang && i < preferred.length; i++) {
            var primary = String(preferred[i]).split('-')[0].toLowerCase();
            if (offered.indexOf(primary) >= 0) lang = primary;
        }
        if (!lang) lang = meta.getAttribute('data-default');

        if (lang && lang !== current && offered.indexOf(lang) >= 0) {
            location.replace('/' + lang + rest + location.search + location.hash);
        }
    } catch (e) {
        /* Stay on the page as served. */
    }
})();
