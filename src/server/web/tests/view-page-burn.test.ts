/**
 * The view page on the burn warning, confirmed first: from that click on the
 * language switch only stores the choice - not only from the retrieve request,
 * which follows the key derivation. A file of its own, as the page defines its
 * custom element once per document.
 */

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import en from '../src/i18n/en.json';
import { LANGUAGE_NEXT_PAGE } from '../src/i18n/language-next-page';
import { LANGUAGE_STORAGE_KEY } from '../src/language';
import { ID, KEY, burnWarningButton, loadBurnCapsule } from './view-burn-page';

let retrieves: string[];

beforeAll(async () => {
    retrieves = await loadBurnCapsule();
});

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page after the burn confirmation', () => {
    it('only stores a language choice made right after the click', async () => {
        burnWarningButton().click();
        // No request yet: the keys are still being derived.
        expect(retrieves).toHaveLength(0);

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="de"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(window.location.pathname).toBe(`/p/${ID}`);
        expect(window.location.hash).toBe(`#${KEY}`);
        expect(document.querySelector('[data-language-notice]')!.textContent).toBe(
            LANGUAGE_NEXT_PAGE.de,
        );

        await vi.waitFor(() => expect(retrieves).toHaveLength(1));
        expect(document.querySelector('cb-view-page')!.textContent).toContain(
            en['view.decrypting'],
        );
    });
});
