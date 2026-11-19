/**
 * The view page under /de/ wired to its real HTML: texts from the catalogue,
 * the report link under the page prefix, and the language switch only storing
 * the choice once the key is gone from the address. Decrypting real capsules
 * is covered by the Playwright suite.
 */

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import { toBase64url } from '@capsulebin/core';

import { shellFor } from '../scripts/shells';
import de from '../src/i18n/de.json';
import { LANGUAGE_STORAGE_KEY } from '../src/language';

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const KEY = `v1.${toBase64url(new Uint8Array(32).fill(7))}`;

async function loadPage(hash: string): Promise<void> {
    const page = new DOMParser().parseFromString(shellFor('p', 'de'), 'text/html');
    document.documentElement.lang = 'de';
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', `/de/p/${ID}${hash}`);
    await import('../src/pages/view');
    await flush();
}

async function flush(): Promise<void> {
    for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

function notice(): Element | null {
    return document.querySelector('[data-language-notice]');
}

// The page defines its custom element, which a document can do only once: one
// page load for the whole file.
beforeAll(async () => {
    localStorage.clear();
    // Every capsule is unknown: the same answer as a wrong key or an expired one.
    vi.stubGlobal(
        'fetch',
        async () => new Response(JSON.stringify({ code: 'not_available' }), { status: 404 }),
    );
    await loadPage(`#${KEY}`);
});

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page in German', () => {
    it('points the report link at this capsule under the page prefix', () => {
        const links = [...document.querySelectorAll<HTMLAnchorElement>('a[href^="/"]')].map((a) =>
            a.getAttribute('href'),
        );
        expect(links).toContain(`/de/report?id=${ID}`);
        for (const href of links) expect(href).toMatch(/^\/de\//);
    });

    it('explains an unavailable capsule from the catalogue', () => {
        expect(document.querySelector('h1')!.textContent).toBe(de['view.unavailable']);
        expect(document.querySelector('cb-view-page .cb-lede')!.textContent).toBe(
            de['view.unavailableLede'],
        );
        expect(document.querySelector('cb-view-page a')!.getAttribute('href')).toBe('/de/');
    });

    it('only stores a language choice once the key is removed, with a notice', () => {
        expect(window.location.hash).toBe('');

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(window.location.pathname).toBe(`/de/p/${ID}`);
        expect(notice()!.getAttribute('role')).toBe('status');
        expect(notice()!.textContent).toBe(de['shell.languageNextPage']);
    });
});
