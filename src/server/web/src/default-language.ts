/**
 * The default interface language: the language the catalogue keys are written
 * for, the fallback of a page without a usable `<html lang>`, and the
 * `data-default` the shell generator (scripts/shells.ts) writes into every shell.
 * Kept apart from `language.ts` so that the generator, which runs in Node.js,
 * can import it without the page's catalogue.
 */

import type { Language } from '@capsulebin/core';

export const DEFAULT_LANGUAGE: Language = 'en';
