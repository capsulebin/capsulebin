/**
 * Checks the output of the per-language builds (scripts/build.ts), which runs it
 * after the last build and fails on any finding:
 *
 * - a page's scripts carry texts of its own language only: no JS reachable from
 *   `dist/<lang>/*.html` contains a text of another language that differs from
 *   its own and is longer than 3 characters;
 * - texts only the shell templates use stay in the HTML: no JS asset names the
 *   key of such a text;
 * - what does not depend on the language is built once: the chunks without texts
 *   (and without imports of chunks with texts) are the same files for every
 *   language and none exists twice.
 */

import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

import type { Catalogue } from './shells.ts';

/** Shorter texts ("OK", "B") are too likely to appear in any code. */
const MIN_TEXT_LENGTH = 4;

/**
 * Strings of the libraries that equal a catalogue text: highlight.js names its
 * plaintext language "Plain text", like the en text `create.plainText`.
 */
const LIBRARY_STRINGS = new Set(['Plain text']);

/**
 * A quoted path of a JS file, as pages and chunks name each other: `src="/assets/x.js"`,
 * `href="/assets/x.js"`, `from"./x.js"`, `import("./x.js")`, `"assets/x.js"`. Any
 * characters but quotes and whitespace, since chunk names carry whatever the
 * bundler puts in (`vendor~p~manage-….js`); only names of files in `assets/` count.
 */
const JS_REFERENCE = /["'`]([^"'`\s]*\.js)["'`]/g;

export interface DistInput {
    /** The full catalogue of every language, by language code. */
    catalogues: Readonly<Record<string, Catalogue>>;
    /** Keys only the shell templates use. */
    templateOnlyKeys: readonly string[];
}

/**
 * Whether `code` has `text` as a string literal of its own (any quote, verbatim or
 * JSON-escaped). A catalogue text is always a whole string in a bundle; as a mere
 * substring, "File" or "Element" would be found in any identifier.
 */
function contains(code: string, text: string): boolean {
    const bodies = new Set([text, JSON.stringify(text).slice(1, -1)]);
    return [...bodies].some((body) =>
        ['"', "'", '`'].some((q) => code.includes(`${q}${body}${q}`)),
    );
}

/**
 * The texts of `other` that may not appear in `lang`'s scripts: worded
 * differently from `lang`'s text for the same key, not a text `lang` has itself
 * (under another key), long enough to search for.
 */
function foreignTexts(catalogues: DistInput['catalogues'], lang: string, other: string): string[] {
    const own = catalogues[lang] ?? {};
    const ownTexts = new Set(Object.values(own));
    return Object.entries(catalogues[other] ?? {})
        .filter(([key, text]) => text !== own[key] && !ownTexts.has(text))
        .map(([, text]) => text)
        .filter((text) => text.length >= MIN_TEXT_LENGTH && !LIBRARY_STRINGS.has(text));
}

/** Returns what is wrong with `distDir`; empty if nothing is. */
export function checkDist(distDir: string, { catalogues, templateOnlyKeys }: DistInput): string[] {
    const assetsDir = join(distDir, 'assets');
    if (!existsSync(assetsDir)) return [`${assetsDir} does not exist`];

    const assets = new Map(
        readdirSync(assetsDir)
            .filter((name) => name.endsWith('.js'))
            .sort()
            .map((name) => [name, readFileSync(join(assetsDir, name), 'utf8')]),
    );
    const references = (code: string): string[] =>
        [...code.matchAll(JS_REFERENCE)]
            .map(([, path = '']) => path.slice(path.lastIndexOf('/') + 1))
            .filter((name) => assets.has(name));

    /** The JS a language's pages load, imports included. */
    const reachable = (lang: string): string[] => {
        const langDir = join(distDir, lang);
        const pages = readdirSync(langDir).filter((name) => name.endsWith('.html'));
        const seen = new Set<string>();
        const queue = pages.flatMap((page) =>
            references(readFileSync(join(langDir, page), 'utf8')),
        );
        for (let name = queue.pop(); name !== undefined; name = queue.pop()) {
            if (seen.has(name)) continue;
            seen.add(name);
            queue.push(...references(assets.get(name) ?? ''));
        }
        return [...seen].sort();
    };

    const languages = Object.keys(catalogues).sort();
    const problems: string[] = [];
    for (const lang of languages) {
        if (!existsSync(join(distDir, lang)))
            problems.push(`${join(distDir, lang)} does not exist`);
    }
    if (problems.length > 0) return problems;

    const pairs = languages.flatMap((lang) =>
        languages.filter((other) => other !== lang).map((other) => [lang, other] as const),
    );
    const allForeign = [
        ...new Set(pairs.flatMap(([lang, other]) => foreignTexts(catalogues, lang, other))),
    ];
    const hasTexts = (code: string): boolean => allForeign.some((text) => contains(code, text));

    // Only its own language in each language's scripts.
    const shared = new Map<string, string[]>();
    for (const lang of languages) {
        const files = reachable(lang);
        if (files.length === 0) problems.push(`${lang}: the pages load no JS from assets/`);
        for (const other of languages.filter((l) => l !== lang)) {
            for (const text of foreignTexts(catalogues, lang, other)) {
                for (const file of files.filter((f) => contains(assets.get(f) ?? '', text))) {
                    problems.push(
                        `${lang}: ${file} contains the ${other} text ${JSON.stringify(text)}`,
                    );
                }
            }
        }
        shared.set(lang, languageFree(files, assets, references, hasTexts));
    }

    // Template-only texts stay in the HTML.
    for (const [file, code] of assets) {
        for (const key of templateOnlyKeys) {
            if (contains(code, key)) {
                problems.push(`${file} names the template-only key ${key}`);
            }
        }
    }

    // Chunks without texts are built once.
    const [first = '', ...rest] = languages;
    const reference = shared.get(first) ?? [];
    if (reference.length === 0) problems.push(`${first}: no chunk is free of texts`);
    for (const lang of rest) {
        const files = shared.get(lang) ?? [];
        if (files.join() !== reference.join()) {
            problems.push(
                `chunks without texts differ: ${first} [${reference.join(', ')}], ${lang} [${files.join(', ')}]`,
            );
        }
    }
    const byContent = new Map<string, string>();
    for (const file of new Set([...shared.values()].flat())) {
        const code = assets.get(file) ?? '';
        const twin = byContent.get(code);
        if (twin !== undefined) problems.push(`${file} and ${twin} are the same chunk`);
        byContent.set(code, file);
    }

    return problems;
}

/** The chunks among `files` that carry no texts and import none that do. */
function languageFree(
    files: readonly string[],
    assets: ReadonlyMap<string, string>,
    references: (code: string) => string[],
    hasTexts: (code: string) => boolean,
): string[] {
    const dependent = new Map<string, boolean>();
    const visit = (file: string): boolean => {
        const known = dependent.get(file);
        if (known !== undefined) return known;
        dependent.set(file, false); // cycles: decided by the other members
        const code = assets.get(file) ?? '';
        const result = hasTexts(code) || references(code).some(visit);
        dependent.set(file, result);
        return result;
    };
    return files.filter((file) => !visit(file));
}
