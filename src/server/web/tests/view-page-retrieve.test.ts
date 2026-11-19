/**
 * The view page while the capsule is being retrieved: the language switch only
 * stores the choice from before the retrieve request on, since loading the page
 * under another prefix then could burn the capsule here and show it nowhere.
 * A file of its own, as the page defines its custom element once per document.
 */

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import { PublicHeader, toBase64url } from '@capsulebin/core';

import { shellFor } from '../scripts/shells';
import de from '../src/i18n/de.json';
import { LANGUAGE_STORAGE_KEY } from '../src/language';

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const KEY = `v1.${toBase64url(new Uint8Array(32).fill(7))}`;

const retrieves: string[] = [];

function descriptor(): object {
    const header = new PublicHeader(new Uint8Array(12), new Uint8Array(16));
    return {
        id: ID,
        format_version: 1,
        public_header: toBase64url(header.toAad()),
        expires_at: null,
        burn_after_retrieval: false,
        payload_size: 1536,
    };
}

async function flush(): Promise<void> {
    for (let i = 0; i < 20; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeAll(async () => {
    localStorage.clear();
    // The descriptor arrives, the retrieval never answers.
    vi.stubGlobal('fetch', async (url: string) => {
        if (!String(url).endsWith('/retrieve')) {
            return new Response(JSON.stringify(descriptor()), { status: 200 });
        }
        retrieves.push(String(url));
        return new Promise<Response>(() => {});
    });

    const page = new DOMParser().parseFromString(shellFor('p', 'de'), 'text/html');
    document.documentElement.lang = 'de';
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', `/de/p/${ID}#${KEY}`);
    await import('../src/pages/view');
    await flush();
});

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page during the retrieval', () => {
    it('only stores a language choice, with a notice', () => {
        expect(retrieves).toHaveLength(1);
        expect(window.location.hash).toBe(`#${KEY}`);

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(window.location.pathname).toBe(`/de/p/${ID}`);
        expect(window.location.hash).toBe(`#${KEY}`);
        expect(document.querySelectorAll('[data-language-notice]')).toHaveLength(1);
        expect(document.querySelector('[data-language-notice]')!.textContent).toBe(
            de['shell.languageNextPage'],
        );
    });
});
