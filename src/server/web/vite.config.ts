import { resolve } from 'node:path';
import { defineConfig, type Plugin } from 'vite';

import {
    GENERATED_DIR,
    SHELLS,
    catalogueLanguages,
    fillServerTokens,
    generateShells,
    generatedPath,
    type ShellName,
} from './scripts/shells.ts';
import { DEFAULT_LANGUAGE } from './src/default-language.ts';

/**
 * The shells are generated per language from the templates in this directory and
 * the catalogues in src/i18n/ (scripts/shells.ts) before Vite sees them. Vite
 * builds the generated files, all languages in one build; this plugin puts them
 * at `dist/<lang>/<shell>.html` and, in the dev server, answers the page routes
 * the way the server does with `DEFAULT_LANGUAGE` as its default language: that
 * language's pages without a prefix, every other under `/<lang>/`, and the
 * shells' server tokens filled in accordingly. Paths under the default
 * language's prefix get a 404 there instead of the server's redirect.
 */
function languageShells(): Plugin {
    /** The shell a page path without language prefix shows, e.g. `/p/x` -> `p`. */
    const shellForPage = (path: string): ShellName | null => {
        if (path === '/') return 'index';
        if (path === '/report') return 'report';
        if (/^\/(imprint|privacy|terms)$/.test(path)) return 'legal';
        if (/^\/p\/[^/]+$/.test(path)) return 'p';
        if (/^\/manage\/[^/]+$/.test(path)) return 'manage';
        return null;
    };
    const shellForRoute = (path: string): { lang: string; shell: ShellName } | null => {
        const match = /^\/([a-z]{2,3})(\/.*)$/.exec(path);
        const [, prefix = '', tail = ''] = match ?? [];
        const prefixed = prefix !== DEFAULT_LANGUAGE && catalogueLanguages().includes(prefix);
        const shell = shellForPage(prefixed ? tail : path);
        return shell ? { lang: prefixed ? prefix : DEFAULT_LANGUAGE, shell } : null;
    };
    /** The language of a generated shell by its dev server path (`/.shells/de/p.html`). */
    const languageOfShell = (path: string): string | null =>
        new RegExp(`^/${GENERATED_DIR}/([^/]+)/`).exec(path)?.[1] ?? null;
    /**
     * Paths that would otherwise get a raw template (`{{lang}}` unfilled): the
     * bare prefix (`/de`) and the templates themselves.
     */
    const wouldServeTemplate = (path: string): boolean =>
        catalogueLanguages().some((lang) => path === `/${lang}`) ||
        SHELLS.some((shell) => path === `/${shell}.html`);

    return {
        name: 'capsulebin-language-shells',
        enforce: 'post',
        config() {
            const shells = generateShells();
            return {
                build: {
                    rolldownOptions: {
                        input: shells.map((path) => resolve(import.meta.dirname, path)),
                    },
                },
            };
        },
        generateBundle(_, bundle) {
            const prefix = `${GENERATED_DIR}/`;
            for (const file of Object.values(bundle)) {
                if (file.fileName.startsWith(prefix)) {
                    file.fileName = file.fileName.slice(prefix.length);
                }
            }
        },
        configureServer(server) {
            const sources = [
                ...SHELLS.map((shell) => resolve(import.meta.dirname, `${shell}.html`)),
                resolve(import.meta.dirname, 'src/i18n'),
            ];
            server.watcher.add(sources);
            server.watcher.on('change', (file) => {
                if (sources.some((source) => file.startsWith(source))) {
                    generateShells();
                    server.ws.send({ type: 'full-reload' });
                }
            });
            server.middlewares.use((req, res, next) => {
                const url = new URL(req.url ?? '/', 'http://dev');
                const route = shellForRoute(url.pathname);
                if (route) {
                    req.url = `/${generatedPath(route.lang, route.shell)}${url.search}`;
                } else if (wouldServeTemplate(url.pathname)) {
                    res.statusCode = 404;
                    res.setHeader('Content-Type', 'text/plain; charset=utf-8');
                    res.end(
                        `Not found. The dev server serves ${DEFAULT_LANGUAGE} pages without a ` +
                            `prefix (/) and every other language under its own (/<lang>/).\n`,
                    );
                    return;
                }
                next();
            });
        },
        transformIndexHtml(html, ctx) {
            // Only the dev server: in a build the tokens stay for the server to fill.
            const lang = ctx.server ? languageOfShell(ctx.path) : null;
            return lang ? fillServerTokens(html, lang, DEFAULT_LANGUAGE) : html;
        },
    };
}

/**
 * Vite is only a dev server and bundler - there is no SPA runtime. Hence a
 * multi-page build: every route gets its own document and the server maps path to
 * file.
 */
export default defineConfig({
    root: import.meta.dirname,
    base: '/',
    // No SPA fallback: an unknown path is a 404, not the raw index.html template.
    appType: 'mpa',
    plugins: [languageShells()],

    build: {
        outDir: 'dist',
        emptyOutDir: true,

        // The browser baseline, stated explicitly rather than inherited from Vite's
        // default, which changes between Vite versions and would move the baseline
        // without anyone deciding to.
        //
        // This has to stay in step with `target` and `lib` in tsconfig.json, because
        // the two enforce different halves of the same promise: the bundler rewrites
        // *syntax* down to these browsers, but it does not polyfill *APIs*. If
        // tsconfig claimed a newer library than the browsers here provide, then
        // `Object.groupBy` or `Set.prototype.union` would type-check cleanly and then
        // throw at runtime for part of the audience. tests/build-target.test.ts fails
        // if the two drift apart.
        target: ['chrome107', 'edge107', 'firefox104', 'safari16'],
        // No inlined assets: a data URI inside a style attribute would be an inline
        // style and thus violate the CSP. 0 forces separate files.
        assetsInlineLimit: 0,
        // No source maps in production: the bundle ships inside the binary and source
        // maps would only widen the attack surface.
        sourcemap: false,
    },

    server: {
        host: '127.0.0.1',
        port: 5173,
        // API and health go to the Axum server. The URL fragment is never sent by the
        // browser anyway, not even through this proxy.
        proxy: {
            '/api': 'http://127.0.0.1:8081',
            '/health': 'http://127.0.0.1:8081',
        },
    },
});
