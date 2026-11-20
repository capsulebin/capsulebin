/**
 * The page language of a unit test, set up the way a generated shell does it:
 * `<html lang>` plus the catalogue of that language, registered with
 * `src/language.ts` as `src/i18n/register-<lang>.ts` does in the browser.
 */

import type { Language } from '@capsulebin/core';

import { DEFAULT_LANGUAGE } from '../src/default-language';
import { type ShellName, fillServerTokens, readCatalogue, shellFor } from '../scripts/shells';

/**
 * The shell `shell` in language `lang` as the server sends it: generated, then the
 * server tokens filled in for an instance with the default language `defaultLanguage`.
 */
export function servedShell(
    shell: ShellName,
    lang: Language,
    defaultLanguage: Language = DEFAULT_LANGUAGE,
): string {
    return fillServerTokens(shellFor(shell, lang), lang, defaultLanguage);
}

/**
 * Makes `lang` the page language. The catalogue goes to the `src/language.ts` the
 * module registry currently holds, so after `vi.resetModules()` call this before
 * importing the page.
 */
export async function setPageLanguage(lang: Language): Promise<void> {
    document.documentElement.lang = lang;
    const { register } = await import('../src/language');
    register(readCatalogue(lang));
}
