/**
 * The catalogues in src/i18n/: every language has every text, plural forms are
 * complete for the language's plural rules, and the catalogue files match the
 * list of known languages in @capsulebin/core.
 */

import { KNOWN_LANGUAGES } from '@capsulebin/core';
import { describe, expect, it } from 'vitest';

import { catalogueLanguages, readCatalogue } from '../scripts/shells';

const PLURAL_CATEGORIES = ['zero', 'one', 'two', 'few', 'many', 'other'];

const languages = catalogueLanguages();

/** Keys with their plural forms (`<base>#<category>`) folded into `<base>#`. */
function normalisedKeys(lang: string): string[] {
    const keys = Object.keys(readCatalogue(lang)).map((key) => key.replace(/#.*$/, '#'));
    return [...new Set(keys)].sort();
}

function pluralBases(lang: string): string[] {
    return normalisedKeys(lang)
        .filter((key) => key.endsWith('#'))
        .map((key) => key.slice(0, -1));
}

describe('catalogues', () => {
    /** The other half of this check is a Rust test against the server's list. */
    it('exist exactly for the languages @capsulebin/core knows', () => {
        expect(languages).toEqual([...KNOWN_LANGUAGES].sort());
    });

    it.each(languages)('%s has the same texts as en', (lang) => {
        expect(normalisedKeys(lang)).toEqual(normalisedKeys('en'));
    });

    it.each(languages)('%s has exactly the plural forms its plural rules use', (lang) => {
        const catalogue = readCatalogue(lang);
        const categories = new Intl.PluralRules(lang).resolvedOptions().pluralCategories;
        const bases = pluralBases(lang);
        expect(bases.length).toBeGreaterThan(0);

        for (const base of bases) {
            const present = PLURAL_CATEGORIES.filter((c) => `${base}#${c}` in catalogue);
            expect(present.sort(), base).toEqual([...categories].sort());
        }
    });

    /** The burn checkbox sits under its label (create page options row). */
    it.each(languages)('%s does not repeat the burn label in the checkbox text', (lang) => {
        const catalogue = readCatalogue(lang);
        const label = catalogue['create.burnLabel']!;
        const text = catalogue['create.burn']!;
        expect(text).not.toBe(label);
        expect(text).not.toContain(label);
        // No second name for the option in brackets either.
        expect(text).not.toMatch(/[()]/);
    });

    it('names the burn option alike in en and de', () => {
        expect(readCatalogue('en')['create.burn']).toBe('Delete after first retrieval');
        expect(readCatalogue('de')['create.burn']).toBe('Nach dem ersten Abruf löschen');
    });

    it.each(languages)('%s has no empty text', (lang) => {
        for (const [key, value] of Object.entries(readCatalogue(lang))) {
            expect(value.trim(), key).not.toBe('');
        }
    });

    /**
     * `{{lang}}` in the templates is the language code; a key without a dot could
     * clash. Only a plural form has a `#<category>` suffix.
     */
    it.each(languages)('%s keys are namespaced', (lang) => {
        for (const key of Object.keys(readCatalogue(lang))) {
            expect(key).toMatch(
                new RegExp(
                    `^[A-Za-z0-9_]+(\\.[A-Za-z0-9_]+)+(#(${PLURAL_CATEGORIES.join('|')}))?$`,
                ),
            );
        }
    });

    /** Same placeholders in every language, or a parameter would silently go missing. */
    it.each(languages)('%s uses the same {parameters} as en', (lang) => {
        const params = (text: string) => [...text.matchAll(/\{\w+\}/g)].map((m) => m[0]).sort();
        const en = readCatalogue('en');
        const catalogue = readCatalogue(lang);
        for (const [key, value] of Object.entries(catalogue)) {
            const reference = en[key] ?? en[key.replace(/#.*$/, '#other')];
            expect(params(value), key).toEqual(params(reference ?? ''));
        }
    });
});
