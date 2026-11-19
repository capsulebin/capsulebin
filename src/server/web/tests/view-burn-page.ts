/**
 * The English view page on the burn warning of a one-time capsule, for the tests
 * of what the language switch and the confirmation do to each other. The page
 * defines its custom element once per document: one load per test file.
 */

import { vi } from 'vitest';

import { PublicHeader, toBase64url } from '@capsulebin/core';

import en from '../src/i18n/en.json';
import { servedShell, setPageLanguage } from './page-language';

export const ID = 'AbCdEfGhIjKlMnOpQrStUv';
export const KEY = `v1.${toBase64url(new Uint8Array(32).fill(7))}`;

function descriptor(): object {
    const header = new PublicHeader(new Uint8Array(12), new Uint8Array(16));
    return {
        id: ID,
        format_version: 1,
        public_header: toBase64url(header.toAad()),
        expires_at: null,
        burn_after_retrieval: true,
        payload_size: 1536,
    };
}

export async function flush(): Promise<void> {
    for (let i = 0; i < 20; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

/**
 * Loads the page up to the burn warning and returns the list the retrieve
 * requests are added to. The descriptor arrives, a retrieval never answers.
 */
export async function loadBurnCapsule(): Promise<string[]> {
    const retrieves: string[] = [];
    localStorage.clear();
    vi.stubGlobal('fetch', async (url: string) => {
        if (!String(url).endsWith('/retrieve')) {
            return new Response(JSON.stringify(descriptor()), { status: 200 });
        }
        retrieves.push(String(url));
        return new Promise<Response>(() => {});
    });

    const page = new DOMParser().parseFromString(servedShell('p', 'en'), 'text/html');
    await setPageLanguage('en');
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', `/p/${ID}#${KEY}`);
    await import('../src/pages/view');
    await flush();
    return retrieves;
}

/** The confirmation button of the burn warning. */
export function burnWarningButton(): HTMLButtonElement {
    const button = [...document.querySelectorAll<HTMLButtonElement>('cb-view-page button')].find(
        (candidate) => candidate.textContent?.trim() === en['view.burnConfirm'],
    );
    if (!button) throw new Error('The page does not show the burn warning.');
    return button;
}
