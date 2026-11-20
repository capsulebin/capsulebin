/* Sets data-theme before the body is painted.
 *
 * Why this file exists separately from src/theme.ts:
 *
 * The Content Security Policy forbids inline scripts, which rules out the usual
 * flash-of-unstyled-content snippet in <head>. A module script would come too
 * late - `type="module"` is always deferred and runs after the document is
 * parsed. An EXTERNAL classic script without defer/async, by contrast, blocks the
 * parser and is guaranteed to run before the first paint. It is self-hosted and
 * therefore covered by `script-src 'self'`.
 *
 * It lives in public/ and Vite copies it to dist/ unchanged - not bundled and
 * without a hash in the name, so that the <script> reference stays stable across
 * all HTML shells.
 *
 * The storage key is duplicated here on purpose (see src/theme.ts). Importing it
 * would turn this file into a module and destroy the very property it exists
 * for. A test keeps both places in sync.
 *
 * It is the first head script and also narrows <meta name="color-scheme"> to an
 * explicit choice. When language-init.js (the next script) moves the page to
 * another language, the current document is never styled; without the meta the
 * browser would paint it in its default light canvas, a bright flash in dark
 * mode. The meta needs no style, so it stays within the CSP. Once the stylesheet
 * applies, `color-scheme` in tokens.css takes precedence over the meta.
 */
(function () {
    try {
        var choice = localStorage.getItem('capsulebin.theme');
        if (choice === 'light' || choice === 'dark') {
            document.documentElement.setAttribute('data-theme', choice);
            var meta = document.querySelector('meta[name="color-scheme"]');
            if (meta) {
                meta.setAttribute('content', choice);
            }
        }
    } catch (e) {
        /* No access to localStorage: prefers-color-scheme decides. */
    }
})();
