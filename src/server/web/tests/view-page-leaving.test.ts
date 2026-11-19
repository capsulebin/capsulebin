/**
 * The view page on the burn warning, language chosen first: the old document
 * stays on screen until the page has loaded in the new language, and a
 * confirmation clicked in that time must not retrieve - the capsule would be
 * burned here and shown nowhere. A file of its own, as the page defines its
 * custom element once per document.
 */

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import { LANGUAGE_STORAGE_KEY } from '../src/language';
import { burnWarningButton, flush, loadBurnCapsule } from './view-burn-page';

let retrieves: string[];

beforeAll(async () => {
    retrieves = await loadBurnCapsule();
});

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page while the language switch loads it in another language', () => {
    it('does not retrieve on the burn confirmation', async () => {
        const confirm = burnWarningButton();

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="de"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(confirm.disabled).toBe(true);

        // Even a click that still gets through starts nothing.
        confirm.disabled = false;
        confirm.click();
        await flush();
        expect(retrieves).toHaveLength(0);
        expect(burnWarningButton()).toBe(confirm);
        expect(document.querySelector('[data-language-notice]')).toBeNull();
    });
});
