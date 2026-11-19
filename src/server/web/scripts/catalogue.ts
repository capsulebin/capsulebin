/**
 * The catalogue the scripts see: `virtual:catalogue`, imported by
 * `src/language.ts`.
 *
 * The catalogue files in `src/i18n/` hold every text of a language, for the shell
 * templates and the scripts alike. The templates take theirs when the shells are
 * generated (scripts/shells.ts); the scripts get a module with the texts the
 * TypeScript in `src/` actually names, nothing else:
 *
 * - in a production build, one module per language (scripts/build.ts runs one
 *   Vite build per language), so a page's scripts carry its own language only;
 * - in the dev server and the unit tests, one module with every language that
 *   answers for `<html lang>`, so pages under `/en/` and `/de/` share one module
 *   graph and tests can switch the page language.
 *
 * A text the scripts need that a catalogue lacks fails the build.
 */

import { readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { Plugin } from 'vite';

import { type Catalogue, catalogueLanguages, readCatalogue } from './shells.ts';

const SOURCE_DIR = resolve(import.meta.dirname, '..', 'src');

/** Catalogue keys are dotted names; a plural form adds `#<category>`. */
const KEY_LITERAL = /(['"`])([A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+)\1/g;

const MODULE_ID = 'virtual:catalogue';
const RESOLVED_ID = `\0${MODULE_ID}`;

function pluralBase(key: string): string {
    return key.replace(/#.*$/, '');
}

function sourceFiles(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true, recursive: true })
        .filter((entry) => entry.isFile() && entry.name.endsWith('.ts'))
        .map((entry) => join(entry.parentPath, entry.name))
        .sort();
}

/**
 * The keys (plural forms by their base) the scripts name: every string literal
 * in `src/**\/*.ts` that is a key of some catalogue. Keys are only ever written
 * out whole (`TextKey`/`PluralKey` are literal types), so this finds all of them;
 * a literal that merely looks like a key costs one text, nothing more.
 */
export function scriptKeys(): string[] {
    const known = new Set(
        catalogueLanguages().flatMap((lang) => Object.keys(readCatalogue(lang)).map(pluralBase)),
    );
    const used = new Set<string>();
    for (const file of sourceFiles(SOURCE_DIR)) {
        for (const [, , literal = ''] of readFileSync(file, 'utf8').matchAll(KEY_LITERAL)) {
            if (known.has(literal)) used.add(literal);
        }
    }
    return [...used].sort();
}

/**
 * The texts of `lang` the scripts use: plain keys as they are, plural keys with
 * every form `Intl.PluralRules` has for the language. Throws if one is missing.
 */
export function scriptCatalogue(lang: string, keys: readonly string[] = scriptKeys()): Catalogue {
    const catalogue = readCatalogue(lang);
    const categories = new Intl.PluralRules(lang).resolvedOptions().pluralCategories;
    const result: Record<string, string> = {};
    for (const key of keys) {
        const plural = Object.keys(catalogue).some((k) => k !== key && pluralBase(k) === key);
        for (const name of plural ? categories.map((c) => `${key}#${c}`) : [key]) {
            const value = catalogue[name];
            if (value === undefined) throw new Error(`${lang}.json has no entry for ${name}`);
            result[name] = value;
        }
    }
    return result;
}

/** The keys only the shell templates use: in no script catalogue. */
export function templateOnlyKeys(lang: string): string[] {
    const used = new Set(Object.keys(scriptCatalogue(lang)));
    return Object.keys(readCatalogue(lang)).filter((key) => !used.has(key));
}

/** One language, for a production build. */
function singleLanguageModule(lang: string): string {
    return `export default ${JSON.stringify(scriptCatalogue(lang))};\n`;
}

/**
 * Every language, picked by `<html lang>` on each lookup (dev server and tests
 * only). Unknown or missing `lang` reads as the first language, `en`, as
 * `pageLanguage()` does.
 */
function allLanguagesModule(): string {
    const keys = scriptKeys();
    const catalogues = Object.fromEntries(
        catalogueLanguages().map((lang) => [lang, scriptCatalogue(lang, keys)]),
    );
    return [
        `const catalogues = ${JSON.stringify(catalogues)};`,
        'const current = () =>',
        "    catalogues[document.documentElement.lang.split('-')[0].toLowerCase()] ?? catalogues.en;",
        'export default new Proxy({}, { get: (_, key) => current()[key] });',
        '',
    ].join('\n');
}

/**
 * Provides `virtual:catalogue`: the catalogue of `lang`, or with `null` every
 * catalogue chosen by `<html lang>` (dev server and tests).
 */
export function catalogueModule(lang: string | null): Plugin {
    return {
        name: 'capsulebin-catalogue',
        resolveId(id) {
            return id === MODULE_ID ? RESOLVED_ID : null;
        },
        load(id) {
            if (id !== RESOLVED_ID) return null;
            return lang === null ? allLanguagesModule() : singleLanguageModule(lang);
        },
        // In the dev server, a changed catalogue or script may change the texts.
        handleHotUpdate({ file, server }) {
            if (!file.startsWith(SOURCE_DIR)) return;
            const module = server.moduleGraph.getModuleById(RESOLVED_ID);
            if (module) server.moduleGraph.invalidateModule(module);
        },
    };
}
