/* Moves the page to the preferred interface language before anything is shown.
 *
 * Every page exists once per language: without a path prefix in the default
 * language (/p/...), under a prefix in every other (/de/p/...). The server does
 * not look at the browser; this script picks the language:
 *
 *   1. the choice stored by the language switch (localStorage, if offered),
 *   2. the first entry of navigator.languages whose primary subtag is offered,
 *   3. the default language.
 *
 * If that differs from the language of the current path, it replaces the page
 * with the same path in the other language. Query and fragment are kept
 * unchanged - the fragment holds the key and never leaves the browser.
 *
 * The offered languages come from <meta name="capsulebin-languages">, which the
 * shell generator (scripts/shells.ts) writes into every shell, the default
 * language from its data-default, which the server fills in. Without the meta
 * or a usable default, without localStorage, or on any exception the script
 * does nothing.
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

        var fallback = meta.getAttribute('data-default');
        if (offered.indexOf(fallback) < 0) return;

        var match = /^\/([^/]+)(.*)$/.exec(location.pathname);
        var prefixed = match && offered.indexOf(match[1]) >= 0;
        var current = prefixed ? match[1] : fallback;
        var rest = prefixed ? match[2] || '/' : location.pathname;

        var lang = null;
        if (stored && offered.indexOf(stored) >= 0) {
            lang = stored;
        }
        var preferred = navigator.languages || [];
        for (var i = 0; !lang && i < preferred.length; i++) {
            var primary = String(preferred[i]).split('-')[0].toLowerCase();
            if (offered.indexOf(primary) >= 0) lang = primary;
        }
        if (!lang) lang = fallback;

        if (lang !== current) {
            var prefix = lang === fallback ? '' : '/' + lang;
            location.replace(prefix + rest + location.search + location.hash);
        }
    } catch (e) {
        /* Stay on the page as served. */
    }
})();
