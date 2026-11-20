/**
 * The view page on a capsule without burn warning or password, language chosen
 * while the descriptor is on its way: the retrieval follows without a click, and
 * the old document must not send it - the page loading in the new language
 * retrieves the capsule itself. A file of its own, as the page defines its
 * custom element once per document.
 */

import { afterAll, describe, expect, it, vi } from 'vitest';

import { LANGUAGE_STORAGE_KEY } from '../src/language';
import { flush, loadPlainCapsule } from './view-burn-page';

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page while the language switch loads it in another language', () => {
    it('does not retrieve once the descriptor arrives', async () => {
        let answer = () => {};
        const described = new Promise<void>((resolve) => (answer = resolve));
        const retrieves = await loadPlainCapsule(described);

        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="de"]')!.click();
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');

        answer();
        await flush();
        expect(retrieves).toHaveLength(0);
        expect(document.querySelector('[data-language-notice]')).toBeNull();
    });
});
