// @vitest-environment node
/**
 * Acceptance test for the Vite dev server: it serves the pages the way the server
 * does with `en` as default language - English without a prefix, every other
 * language under its own, the server tokens of the shells filled in - and answers
 * 404, never a raw template with unfilled `{{…}}` placeholders, for the default
 * language's prefix, a bare prefix and the template files.
 */

import { resolve } from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { createServer, type ViteDevServer } from 'vite';

import { KNOWN_LANGUAGES } from '@capsulebin/core';

import { DEFAULT_LANGUAGE } from '../src/default-language';
import { languagePrefix } from '../src/language-prefix';

let server: ViteDevServer;
let origin: string;

beforeAll(async () => {
    server = await createServer({
        configFile: resolve(import.meta.dirname, '../vite.config.ts'),
        logLevel: 'silent',
        server: { port: 0, host: '127.0.0.1', strictPort: false, hmr: false },
    });
    await server.listen();
    const address = server.httpServer?.address();
    if (!address || typeof address === 'string') throw new Error('dev server has no port');
    origin = `http://127.0.0.1:${address.port}`;
}, 60_000);

afterAll(async () => {
    await server?.close();
});

async function get(path: string): Promise<{ status: number; text: string }> {
    const response = await fetch(origin + path, { redirect: 'manual' });
    return { status: response.status, text: await response.text() };
}

const PAGES = [
    '/',
    '/p/AbCdEfGhIjKlMnOpQrStUv',
    '/manage/AbCdEfGhIjKlMnOpQrStUv',
    '/report?id=x',
    '/imprint',
    '/privacy',
    '/terms',
];

describe('dev server', () => {
    it('serves every page of every language with a filled shell', async () => {
        for (const lang of KNOWN_LANGUAGES) {
            const prefix = languagePrefix(DEFAULT_LANGUAGE, lang);
            for (const page of PAGES) {
                const path = prefix + page;
                const answer = await get(path);
                expect(answer.status, path).toBe(200);
                expect(answer.text, path).toContain(`<html lang="${lang}"`);
                expect(answer.text, path).not.toContain('{{');
                expect(answer.text, path).not.toContain('%%');
                expect(answer.text, path).toContain(`data-default="${DEFAULT_LANGUAGE}"`);
                expect(answer.text, path).toContain(`<a class="cb-logo" href="${prefix}/">`);
            }
        }
    });

    it('answers 404 under the prefix of the default language', async () => {
        for (const page of PAGES) {
            const path = `/${DEFAULT_LANGUAGE}${page}`;
            const answer = await get(path);
            expect(answer.status, path).toBe(404);
            expect(answer.text, path).not.toContain('{{');
        }
    });

    it('answers 404 instead of a raw template', async () => {
        for (const path of [
            '/index.html',
            '/p.html',
            '/manage.html',
            '/report.html',
            '/legal.html',
            ...KNOWN_LANGUAGES.map((lang) => `/${lang}`),
        ]) {
            const answer = await get(path);
            expect(answer.status, path).toBe(404);
            expect(answer.text, path).not.toContain('{{');
        }
    });

    it('has no SPA fallback for an unknown path', async () => {
        const answer = await get('/does/not/exist');
        expect(answer.status).toBe(404);
        expect(answer.text).not.toContain('{{');
    });
});
