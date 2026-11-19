// @vitest-environment node
/**
 * Acceptance test for the external review fix "dev server 404": the Vite dev
 * server serves the pages under every language prefix like the server does and
 * answers 404 - never a raw template with unfilled `{{…}}` placeholders - for
 * page paths without a prefix, the bare prefix and the template files.
 */

import { resolve } from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { createServer, type ViteDevServer } from 'vite';

import { KNOWN_LANGUAGES } from '@capsulebin/core';

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

describe('dev server', () => {
    it('serves every page under every language prefix with a filled shell', async () => {
        for (const lang of KNOWN_LANGUAGES) {
            for (const path of [
                '/',
                '/p/AbCdEfGhIjKlMnOpQrStUv',
                '/manage/AbCdEfGhIjKlMnOpQrStUv',
                '/report?id=x',
                '/imprint',
                '/privacy',
                '/terms',
            ]) {
                const answer = await get(`/${lang}${path}`);
                expect(answer.status, `/${lang}${path}`).toBe(200);
                expect(answer.text, `/${lang}${path}`).toContain(`<html lang="${lang}"`);
                expect(answer.text, `/${lang}${path}`).not.toContain('{{');
            }
        }
    });

    it('answers 404 instead of a raw template', async () => {
        for (const path of [
            '/',
            '/p/AbCdEfGhIjKlMnOpQrStUv',
            '/manage/AbCdEfGhIjKlMnOpQrStUv',
            '/report',
            '/report?id=x',
            '/imprint',
            '/privacy',
            '/terms',
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
