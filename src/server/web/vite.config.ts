import { resolve } from 'node:path';
import { defineConfig } from 'vite';

/**
 * Vite is only a dev server and bundler - there is no SPA runtime. Hence a
 * multi-page build: every route gets its own document and the server maps path to
 * file.
 */
export default defineConfig({
    root: import.meta.dirname,

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
            input: {
                index: resolve(import.meta.dirname, 'index.html'),
                p: resolve(import.meta.dirname, 'p.html'),
                manage: resolve(import.meta.dirname, 'manage.html'),
                report: resolve(import.meta.dirname, 'report.html'),
            },
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
