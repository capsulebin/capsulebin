// @vitest-environment node
//
// Reads the sources and catalogues from disk and needs no DOM.

/**
 * scripts/catalogue.ts: the scripts get the texts they name and nothing else,
 * every language has all of them, and a missing one fails the build.
 */

import { describe, expect, it } from 'vitest';

import { scriptCatalogue, scriptKeys, templateOnlyKeys } from '../scripts/catalogue';
import { catalogueLanguages, readCatalogue } from '../scripts/shells';

const LANGUAGES = catalogueLanguages();

describe('script keys', () => {
    it('are the keys the TypeScript names, plural keys by their base', () => {
        const keys = scriptKeys();
        expect(keys).toContain('expiry.none');
        expect(keys).toContain('view.cancel');
        expect(keys).toContain('expiry.inMinutes');
        expect(keys).not.toContain('manage.cancel');
    });
});

describe.each(LANGUAGES)('%s script catalogue', (lang) => {
    const catalogue = readCatalogue(lang);
    const script = scriptCatalogue(lang);

    it("holds the language's own texts", () => {
        for (const [key, text] of Object.entries(script)) expect(text, key).toBe(catalogue[key]);
    });

    it('has every plural form of the language', () => {
        for (const category of new Intl.PluralRules(lang).resolvedOptions().pluralCategories) {
            expect(script).toHaveProperty([`expiry.inMinutes#${category}`]);
        }
    });

    it('leaves out the texts only the shell templates use', () => {
        const templateOnly = templateOnlyKeys(lang);
        expect(templateOnly).toContain('manage.cancel');
        for (const key of templateOnly) expect(script).not.toHaveProperty([key]);
        expect(Object.keys(script).length + templateOnly.length).toBe(
            Object.keys(catalogue).length,
        );
    });
});

describe('a text the scripts need', () => {
    it('fails the build if a catalogue lacks it', () => {
        expect(() => scriptCatalogue('de', ['no.such'])).toThrow(
            /de\.json has no entry for no\.such/,
        );
    });
});
