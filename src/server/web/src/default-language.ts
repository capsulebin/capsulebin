/**
 * The built-in default interface language: the language the catalogue keys are
 * written for, the fallback of a page without a usable `<html lang>` or
 * `data-default`, and the default language of the Vite dev server. An instance's
 * own default language is configuration; the server puts it into every shell.
 * Kept apart from `language.ts` so that `vite.config.ts`, which runs in Node.js,
 * can import it without the page's modules.
 */

import type { Language } from '@capsulebin/core';

export const DEFAULT_LANGUAGE: Language = 'en';
