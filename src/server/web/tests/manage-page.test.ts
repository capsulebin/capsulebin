/**
 * The management page under /de/ wired to its real HTML: lifecycle texts and
 * formatting in the page language, error texts from the catalogue, and the
 * language switch only storing the choice once the delete secret is gone from
 * the address.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PublicHeader, toBase64url } from '@capsulebin/core';

import { shellFor } from '../scripts/shells';
import de from '../src/i18n/de.json';
import { LANGUAGE_STORAGE_KEY } from '../src/language';

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const SECRET = `v1.${toBase64url(new Uint8Array(32).fill(7))}`;
const EXPIRES_AT = Date.UTC(2026, 11, 24, 18, 30) / 1000;

function descriptor(): object {
    const header = new PublicHeader(new Uint8Array(12), new Uint8Array(16));
    return {
        id: ID,
        format_version: 1,
        public_header: toBase64url(header.toAad()),
        expires_at: EXPIRES_AT,
        burn_after_retrieval: true,
        payload_size: 1536,
    };
}

function stubServer(describe: () => Response): void {
    vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) =>
        init?.method === 'DELETE' ? new Response(null, { status: 204 }) : describe(),
    );
}

async function loadPage(hash = `#${SECRET}`): Promise<void> {
    const page = new DOMParser().parseFromString(shellFor('manage', 'de'), 'text/html');
    document.documentElement.lang = 'de';
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', `/de/manage/${ID}${hash}`);
    vi.resetModules();
    await import('../src/pages/manage');
    await flush();
}

async function flush(): Promise<void> {
    for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

function text(attr: string): string {
    return document.querySelector(`[data-${attr}]`)!.textContent!.trim();
}

beforeEach(() => {
    localStorage.clear();
    stubServer(() => new Response(JSON.stringify(descriptor()), { status: 200 }));
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('management page in German', () => {
    it('describes the capsule in the page language', async () => {
        await loadPage();
        const date = new Intl.DateTimeFormat('de', {
            dateStyle: 'medium',
            timeStyle: 'short',
        }).format(new Date(EXPIRES_AT * 1000));
        expect(text('meta-expires')).toBe(`Läuft ab am ${date}`);
        expect(text('meta-burn')).toBe(de['manage.burn']);
        expect(text('meta-size')).toBe('1,5 KiB Chiffretext');
    });

    it('only stores a language choice after deleting, with a notice', async () => {
        await loadPage();
        const english = () =>
            document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!;
        expect(document.querySelector('[data-language-notice]')).toBeNull();

        document.querySelector<HTMLButtonElement>('[data-delete-btn]')!.click();
        await flush();
        expect(document.querySelector('h1')!.textContent).toBe(de['manage.deleted']);
        expect(window.location.hash).toBe('');

        english().click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(window.location.pathname).toBe(`/de/manage/${ID}`);
        const notice = document.querySelector('[data-language-notice]')!;
        expect(notice.getAttribute('role')).toBe('status');
        expect(notice.textContent).toBe(de['shell.languageNextPage']);
    });

    it('only stores a language choice while the delete is running', async () => {
        vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) =>
            init?.method === 'DELETE'
                ? new Promise<Response>(() => {})
                : new Response(JSON.stringify(descriptor()), { status: 200 }),
        );
        await loadPage();

        document.querySelector<HTMLButtonElement>('[data-delete-btn]')!.click();
        await flush();
        expect(window.location.hash).toBe(`#${SECRET}`);

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(window.location.pathname).toBe(`/de/manage/${ID}`);
        expect(document.querySelector('[data-language-notice]')!.textContent).toBe(
            de['shell.languageNextPage'],
        );
    });

    it('explains a link without a key', async () => {
        await loadPage('');
        expect(text('error-message')).toBe(de['link.noKey']);
    });

    it('explains a rate limit without the server text', async () => {
        stubServer(
            () =>
                new Response(JSON.stringify({ code: 'rate_limited', title: 'Too many requests' }), {
                    status: 429,
                    headers: { 'Retry-After': '30' },
                }),
        );
        await loadPage();
        expect(text('error-message')).toBe(
            'Zu viele Anfragen. Versuch es in 30 Sekunden noch einmal.',
        );
    });
});
