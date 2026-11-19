import { resolve } from 'node:path';
import { defineConfig, type Plugin } from 'vite';

import {
    GENERATED_DIR,
    SHELLS,
    catalogueLanguages,
    generateShells,
    generatedPath,
    type ShellName,
} from './scripts/shells';

/**
 * The shells are generated per language from the templates in this directory and
 * the catalogues in src/i18n/ (scripts/shells.ts) before Vite sees them. Vite
 * builds the generated files; this plugin puts them at `dist/<lang>/<shell>.html`
 * and, in the dev server, answers the page routes under every language prefix
 * the way the server does. The bundles under /assets/ are the same for every
 * language.
 */
function languageShells(): Plugin {
    const shellForRoute = (path: string): { lang: string; shell: ShellName } | null => {
        const match = /^\/([a-z]{2,3})(\/.*)$/.exec(path);
        const [, lang = '', rest = ''] = match ?? [];
        if (!catalogueLanguages().includes(lang)) return null;
        if (rest === '/') return { lang, shell: 'index' };
        if (rest === '/report') return { lang, shell: 'report' };
        if (/^\/(imprint|privacy|terms)$/.test(rest)) return { lang, shell: 'legal' };
        if (/^\/p\/[^/]+$/.test(rest)) return { lang, shell: 'p' };
        if (/^\/manage\/[^/]+$/.test(rest)) return { lang, shell: 'manage' };
        return null;
    };

    return {
        name: 'capsulebin-language-shells',
        enforce: 'post',
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
            server.middlewares.use((req, _res, next) => {
                const url = new URL(req.url ?? '/', 'http://dev');
                const route = shellForRoute(url.pathname);
                if (route) req.url = `/${generatedPath(route.lang, route.shell)}${url.search}`;
                next();
            });
        },
    };
}

const shells = generateShells();

/**
 * Vite is only a dev server and bundler - there is no SPA runtime. Hence a
 * multi-page build: every route gets its own document and the server maps path to
 * file.
 */
export default defineConfig({
    root: import.meta.dirname,
    base: '/',
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
        rolldownOptions: {
            input: shells.map((path) => resolve(import.meta.dirname, path)),
        },
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
