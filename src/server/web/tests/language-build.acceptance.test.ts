// @vitest-environment node
//
// Runs the production build and reads its output from disk; needs no DOM.

/**
 * Acceptance test for AC22: every page loads only its own language.
 *
 * Runs `npm run build` (scripts/build.ts) and checks the real `dist/` on its own
 * terms, independently of scripts/check-dist.ts:
 *
 * (a) no JS a page under `dist/<lang>/` loads (directly, by modulepreload or by
 *     import, transitively) contains a text of another catalogue that differs from
 *     its own and is longer than 3 characters;
 * (b) keys only the shell templates use appear in no JS asset;
 * (c) chunks without texts exist once in `dist/assets/`: no two JS files have the
 *     same content, and every text-free chunk one language loads is the very same
 *     file the other languages load where they need that code.
 */

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

import { catalogueLanguages, readCatalogue, readTemplate, SHELLS } from '../scripts/shells';

const WEB_ROOT = resolve(import.meta.dirname, '..');
const DIST = join(WEB_ROOT, 'dist');
const ASSETS = join(DIST, 'assets');
const LANGUAGES = catalogueLanguages();

/** Any file name ending in `.js`, including Rolldown's `vendor~a~b-hash.js`. */
const JS_NAME = /[A-Za-z0-9_~.$-]+\.js/g;

function jsAssets(): Map<string, string> {
    return new Map(
        readdirSync(ASSETS)
            .filter((name) => name.endsWith('.js'))
            .sort()
            .map((name) => [name, readFileSync(join(ASSETS, name), 'utf8')]),
    );
}

/** Names of asset JS files mentioned anywhere in `code`. */
function mentioned(code: string, assets: ReadonlyMap<string, string>): string[] {
    return [...new Set(code.match(JS_NAME) ?? [])].filter((name) => assets.has(name));
}

/** Every asset JS file the pages of `lang` load, imports followed transitively. */
function loadedBy(lang: string, assets: ReadonlyMap<string, string>): Set<string> {
    const dir = join(DIST, lang);
    const queue = readdirSync(dir)
        .filter((name) => name.endsWith('.html'))
        .flatMap((page) => mentioned(readFileSync(join(dir, page), 'utf8'), assets));
    const seen = new Set<string>();
    for (let name = queue.pop(); name !== undefined; name = queue.pop()) {
        if (seen.has(name)) continue;
        seen.add(name);
        queue.push(...mentioned(assets.get(name) ?? '', assets));
    }
    return seen;
}

/** Whether `code` holds `text` as a string literal (any quote, raw or JSON-escaped). */
function holdsLiteral(code: string, text: string): boolean {
    const bodies = [text, JSON.stringify(text).slice(1, -1)];
    return bodies.some((body) => ['"', "'", '`'].some((q) => code.includes(`${q}${body}${q}`)));
}

/** Texts of `other` that differ from `lang`'s text for the same key, longer than 3 characters. */
function foreignTexts(lang: string, other: string): Map<string, string> {
    const own = readCatalogue(lang);
    const ownTexts = new Set(Object.values(own));
    return new Map(
        Object.entries(readCatalogue(other)).filter(
            ([key, text]) => text !== own[key] && text.length > 3 && !ownTexts.has(text),
        ),
    );
}

/** Keys a shell template names as `{{key}}` and no TypeScript source does. */
function templateOnlyKeys(): string[] {
    const inTemplates = new Set(
        SHELLS.flatMap((shell) =>
            [...readTemplate(shell).matchAll(/\{\{\s*([\w.#]+)\s*\}\}/g)].map(([, k = '']) => k),
        ),
    );
    const sources = readdirSync(join(WEB_ROOT, 'src'), { recursive: true, withFileTypes: true })
        .filter((e) => e.isFile() && e.name.endsWith('.ts'))
        .map((e) => readFileSync(join(e.parentPath, e.name), 'utf8'))
        .join('\n');
    return [...inTemplates]
        .filter((key) => key.includes('.'))
        .filter((key) => !sources.includes(`'${key}'`) && !sources.includes(`"${key}"`))
        .filter((key) => !sources.includes(`\`${key}\``));
}

beforeAll(() => {
    execFileSync(process.execPath, ['scripts/build.ts'], { cwd: WEB_ROOT, stdio: 'pipe' });
}, 120_000);

describe('AC22: the per-language build', () => {
    it('writes every shell for every language and serves its scripts from /assets/', () => {
        expect(LANGUAGES).toEqual(expect.arrayContaining(['en', 'de']));
        const assets = jsAssets();
        for (const lang of LANGUAGES) {
            for (const shell of SHELLS) {
                const html = readFileSync(join(DIST, lang, `${shell}.html`), 'utf8');
                expect(html, `${lang}/${shell}`).toContain(`<html lang="${lang}"`);
                const srcs = [...html.matchAll(/<script[^>]*\bsrc="([^"]+)"/g)].map(
                    ([, src = '']) => src,
                );
                const modules = srcs.filter(
                    (src) => src.endsWith('.js') && src.includes('/assets/'),
                );
                expect(modules.length, `${lang}/${shell} module script`).toBeGreaterThan(0);
                for (const src of modules) {
                    expect(src, `${lang}/${shell}`).toMatch(/^\/assets\/[^/]+\.js$/);
                    expect(assets.has(src.slice('/assets/'.length)), src).toBe(true);
                }
            }
        }
    });

    it.each(LANGUAGES)('(a) the scripts of %s pages carry no text of another language', (lang) => {
        const assets = jsAssets();
        const loaded = loadedBy(lang, assets);
        expect(loaded.size).toBeGreaterThan(0);
        const findings: string[] = [];
        for (const other of LANGUAGES.filter((l) => l !== lang)) {
            for (const [key, text] of foreignTexts(lang, other)) {
                for (const file of loaded) {
                    if (holdsLiteral(assets.get(file) ?? '', text)) {
                        findings.push(`${file}: ${other} ${key} = ${JSON.stringify(text)}`);
                    }
                }
            }
        }
        // highlight.js names its plaintext language "Plain text", the en text of
        // create.plainText; that string comes from the library, not a catalogue.
        expect(findings.filter((f) => !f.endsWith('"Plain text"'))).toEqual([]);
    });

    it.each(LANGUAGES)('(a) the scripts of %s pages do carry its own texts', (lang) => {
        // Guards the check above against passing on a build without any texts.
        const assets = jsAssets();
        const code = [...loadedBy(lang, assets)].map((f) => assets.get(f) ?? '').join('\n');
        const own = Object.values(readCatalogue(lang)).filter((text) => text.length > 3);
        expect(own.some((text) => holdsLiteral(code, text))).toBe(true);
    });

    it('(b) keys used only in the shell templates are in no JS asset', () => {
        const keys = templateOnlyKeys();
        expect(keys.length).toBeGreaterThan(0);
        const findings: string[] = [];
        for (const [file, code] of jsAssets()) {
            for (const key of keys) {
                // As a whole string, so `create.remove` is not found in `create.removeItem`.
                if (holdsLiteral(code, key)) findings.push(`${file}: ${key}`);
            }
        }
        expect(findings).toEqual([]);
    });

    it('(c) no JS chunk exists twice in dist/assets', () => {
        const byContent = new Map<string, string[]>();
        for (const [file, code] of jsAssets()) {
            byContent.set(code, [...(byContent.get(code) ?? []), file]);
        }
        const twins = [...byContent.values()].filter((files) => files.length > 1);
        expect(twins).toEqual([]);
    });

    it('(c) chunks without texts are shared between the languages', () => {
        const assets = jsAssets();
        const allTexts = [
            ...new Set(
                LANGUAGES.flatMap((lang) =>
                    LANGUAGES.filter((o) => o !== lang).flatMap((o) => [
                        ...foreignTexts(lang, o).values(),
                    ]),
                ),
            ),
        ].filter((text) => text !== 'Plain text');
        const hasTexts = new Map<string, boolean>();
        const dependsOnLanguage = (file: string, stack = new Set<string>()): boolean => {
            const known = hasTexts.get(file);
            if (known !== undefined) return known;
            if (stack.has(file)) return false;
            stack.add(file);
            const code = assets.get(file) ?? '';
            const result =
                allTexts.some((text) => holdsLiteral(code, text)) ||
                mentioned(code, assets).some((dep) => dependsOnLanguage(dep, stack));
            hasTexts.set(file, result);
            return result;
        };

        const free = new Map(
            LANGUAGES.map((lang) => [
                lang,
                [...loadedBy(lang, assets)].filter((f) => !dependsOnLanguage(f)).sort(),
            ]),
        );
        // The libraries are text-free and include the vendor~… chunks.
        const [first = '', ...rest] = LANGUAGES;
        expect(free.get(first)?.some((f) => f.startsWith('vendor'))).toBe(true);
        for (const lang of rest) expect(free.get(lang), lang).toEqual(free.get(first));

        // And every chunk depending on the language exists once per language, not more.
        const dependent = [...assets.keys()].filter((f) => dependsOnLanguage(f));
        for (const file of dependent) {
            const users = LANGUAGES.filter((lang) => loadedBy(lang, assets).has(file));
            expect(users, file).toHaveLength(1);
        }
    });

    it('leaves no catalogue module with both languages in the build', () => {
        expect(existsSync(join(DIST, 'index.html'))).toBe(false);
        const assets = jsAssets();
        for (const lang of LANGUAGES) {
            const code = [...loadedBy(lang, assets)].map((f) => assets.get(f) ?? '').join('\n');
            expect(code, lang).not.toMatch(/["'`]?(de|en)["'`]?\s*:\s*\{\s*["'`][\w]+\.[\w.#]+/);
        }
    });
});
