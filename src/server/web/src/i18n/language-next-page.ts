/**
 * The catalogue text `shell.languageNextPage` in every language: the notice the
 * language switch shows where it only stores the choice. It appears in the
 * chosen language, not the page's, so every page carries this one text in all
 * languages - the only text a page holds outside its own catalogue module.
 *
 * The catalogue files stay the source: a page script cannot import them (a
 * whole catalogue would end up in a chunk every page loads), so the texts are
 * repeated here and tests/catalogue-modules.test.ts fails unless they equal the
 * catalogue entries.
 */

import type { Language } from '@capsulebin/core';

export const LANGUAGE_NEXT_PAGE: Readonly<Record<Language, string>> = {
    en: 'The language applies from the next page on.',
    de: 'Die Sprache gilt ab der nächsten Seite.',
};
