// @vitest-environment node
//
// Reads the sources, runs the production build and reads its output from disk;
// needs no DOM.

/**
 * AC22: every page loads only its own language, with a single web build. The
 * catalogue of a language is the module `src/i18n/register-<lang>.ts`; the
 * generated shells of that language load it before the page script, and nothing
 * else imports a catalogue file.
 *
 * AC27, the one exception: the notice of the language switch appears in the
 * chosen language, so `src/i18n/language-next-page.ts` holds that text - and no
 * other - in every language for every page.
 */

import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { beforeAll, describe, expect, it } from 'vitest';

import { KNOWN_LANGUAGES } from '@capsulebin/core';

import { SHELLS, readCatalogue, shellFor } from '../scripts/shells';
import { LANGUAGE_NEXT_PAGE } from '../src/i18n/language-next-page';

const WEB_ROOT = resolve(import.meta.dirname, '..');
const SOURCES = join(WEB_ROOT, 'src');
const DIST = join(WEB_ROOT, 'dist');
const ASSETS = join(DIST, 'assets');

/** The one catalogue key whose text every page holds in every language (AC27). */
const EVERY_PAGE_KEY = 'shell.languageNextPage';

/** `import … from '….json'`, also a bare or dynamic import; `import type` is erased. */
const JSON_IMPORT = /\bimport\b(?!\s+type\b)[^;'"]*['"]([^'"]+\.json)['"]/g;

/** Source files that import a JSON file at runtime, with the file names they import. */
function jsonImports(): [file: string, imported: string[]][] {
    return readdirSync(SOURCES, { withFileTypes: true, recursive: true })
        .filter((entry) => entry.isFile() && entry.name.endsWith('.ts'))
        .map((entry) => join(entry.parentPath, entry.name))
        .sort()
        .map((file): [string, string[]] => [
            relative(SOURCES, file),
            [...readFileSync(file, 'utf8').matchAll(JSON_IMPORT)].map(([, path = '']) => path),
        ])
        .filter(([, imported]) => imported.length > 0);
}

describe('catalogue modules in the sources', () => {
    it('are the only importers of a catalogue file, each of its own', () => {
        expect(jsonImports()).toEqual(
            [...KNOWN_LANGUAGES]
                .sort()
                .map((lang) => [`i18n/register-${lang}.ts`, [`./${lang}.json`]]),
        );
    });

    it('share only the notice of the language switch with every page', () => {
        expect(LANGUAGE_NEXT_PAGE).toEqual(
            Object.fromEntries(
                KNOWN_LANGUAGES.map((lang) => [lang, readCatalogue(lang)[EVERY_PAGE_KEY]]),
            ),
        );
    });

    it.each(SHELLS)('are loaded by %s.html before the page script', (shell) => {
        for (const lang of KNOWN_LANGUAGES) {
            const modules = [
                ...shellFor(shell, lang).matchAll(/<script type="module" src="([^"]+)">/g),
            ].map(([, src = '']) => src);
            expect(modules, `${lang}/${shell}`).toHaveLength(2);
            expect(modules[0], `${lang}/${shell}`).toBe(`/src/i18n/register-${lang}.ts`);
            expect(modules[1], `${lang}/${shell}`).toMatch(/^\/src\/pages\/[a-z]+\.ts$/);
        }
    });
});

describe('catalogue modules in the build output', () => {
    /** The JS files in `dist/assets`, by name. */
    let assets: Map<string, string>;

    beforeAll(() => {
        execFileSync('npm', ['run', 'build'], { cwd: WEB_ROOT, stdio: 'pipe' });
        assets = new Map(
            readdirSync(ASSETS)
                .filter((name) => name.endsWith('.js'))
                .sort()
                .map((name) => [name, readFileSync(join(ASSETS, name), 'utf8')]),
        );
    }, 120_000);

    /** The asset that is the catalogue module of `lang`. */
    function catalogueAsset(lang: string): string {
        const names = [...assets.keys()].filter((name) => name.startsWith(`register-${lang}-`));
        expect(names, `catalogue module of ${lang}`).toHaveLength(1);
        return names[0]!;
    }

    /** Names of JS assets mentioned anywhere in `text` (script, preload or import). */
    function mentioned(text: string): string[] {
        return [...assets.keys()].filter((name) => text.includes(name));
    }

    /** Every JS asset the page loads, imports followed transitively. */
    function loadedBy(html: string): Set<string> {
        const queue = mentioned(html);
        const seen = new Set<string>();
        for (let name = queue.pop(); name !== undefined; name = queue.pop()) {
            if (seen.has(name)) continue;
            seen.add(name);
            queue.push(...mentioned(assets.get(name) ?? ''));
        }
        return seen;
    }

    /** Texts of `lang` that no other catalogue has, long enough not to occur by chance. */
    function ownTexts(lang: string): string[] {
        const others = new Set(
            KNOWN_LANGUAGES.filter((other) => other !== lang).flatMap((other) =>
                Object.values(readCatalogue(other)),
            ),
        );
        return Object.values(readCatalogue(lang)).filter(
            (text) => text.length > 3 && !others.has(text),
        );
    }

    /** Whether `code` holds `text` as a string literal (any quote, raw or JSON-escaped). */
    function holdsLiteral(code: string, text: string): boolean {
        const bodies = [text, JSON.stringify(text).slice(1, -1)];
        return bodies.some((body) => ['"', "'", '`'].some((q) => code.includes(`${q}${body}${q}`)));
    }

    it.each(KNOWN_LANGUAGES)(
        'every %s page loads its own catalogue module and no other',
        (lang) => {
            const own = catalogueAsset(lang);
            const foreign = KNOWN_LANGUAGES.filter((other) => other !== lang).map(catalogueAsset);
            for (const shell of SHELLS) {
                const html = readFileSync(join(DIST, lang, `${shell}.html`), 'utf8');
                const loaded = loadedBy(html);
                expect(html, `${lang}/${shell}`).toContain(`<html lang="${lang}"`);
                expect(loaded.has(own), `${lang}/${shell} loads ${own}`).toBe(true);
                for (const other of foreign) {
                    expect(loaded.has(other), `${lang}/${shell} loads ${other}`).toBe(false);
                }
            }
        },
    );

    it.each(KNOWN_LANGUAGES)('every page holds the language switch notice in %s', (lang) => {
        const text = readCatalogue(lang)[EVERY_PAGE_KEY]!;
        for (const pageLang of KNOWN_LANGUAGES) {
            for (const shell of SHELLS) {
                const html = readFileSync(join(DIST, pageLang, `${shell}.html`), 'utf8');
                const holders = [...loadedBy(html)].filter(
                    (name) =>
                        !name.startsWith('register-') && holdsLiteral(assets.get(name)!, text),
                );
                expect(holders, `${pageLang}/${shell}`).toHaveLength(1);
            }
        }
    });

    it.each(KNOWN_LANGUAGES)('the texts of %s are in its catalogue module only', (lang) => {
        const own = catalogueAsset(lang);
        const everyPage = readCatalogue(lang)[EVERY_PAGE_KEY];
        const texts = ownTexts(lang).filter((text) => text !== everyPage);
        expect(texts.length).toBeGreaterThan(0);

        const findings: string[] = [];
        for (const [name, code] of assets) {
            for (const text of texts) {
                if (holdsLiteral(code, text) !== (name === own)) findings.push(`${name}: ${text}`);
            }
        }
        // highlight.js names its plaintext language "Plain text", the en text of
        // create.plainText; that string comes from the library, not a catalogue.
        expect(findings.filter((finding) => !finding.endsWith(': Plain text'))).toEqual([]);
    });

    /**
     * AC7, AC6, AC12, AC14 on what the build really writes: the server embeds
     * `dist/<lang>/<shell>.html`, not the generator's output, so whatever Vite does
     * to a shell on the way must leave the parts the server and the head script
     * rely on untouched.
     */
    describe.each(KNOWN_LANGUAGES.flatMap((lang) => SHELLS.map((shell) => [lang, shell] as const)))(
        'built dist/%s/%s.html',
        (lang, shell) => {
            const built = (): string => readFileSync(join(DIST, lang, `${shell}.html`), 'utf8');
            const head = (): string => built().slice(0, built().indexOf('</head>'));
            const count = (text: string, part: string): number => text.split(part).length - 1;

            it('declares its language and has no placeholder or catalogue key left', () => {
                const html = built();
                expect(html).toMatch(new RegExp(`<html lang="${lang}"[ >]`));
                expect(html).not.toMatch(/\{\{|\}\}/);
                for (const key of Object.keys(readCatalogue(lang))) {
                    expect(html, key).not.toContain(key);
                }
            });

            it("keeps the server's markers: legal title and content, one footer link per page", () => {
                const html = built();
                const catalogue = readCatalogue(lang);
                expect(count(html, '<!--capsulebin:legal-title-->')).toBe(
                    shell === 'legal' ? 2 : 0,
                );
                expect(count(html, '<!--capsulebin:legal-content-->')).toBe(
                    shell === 'legal' ? 1 : 0,
                );
                expect(count(html, '<!--capsulebin:legal-link:')).toBe(3);
                const footer = html.slice(html.indexOf('<footer'), html.indexOf('</footer>'));
                for (const page of ['imprint', 'privacy', 'terms']) {
                    expect(footer, page).toContain(
                        `<!--capsulebin:legal-link:${page}:${catalogue[`shell.${page}`]}-->`,
                    );
                }
                expect(html).not.toContain('<!--capsulebin:languages-->');
            });

            it('names every known language in the head, before the head script', () => {
                const metas = [...head().matchAll(/<meta name="capsulebin-languages"[^>]*>/g)];
                expect(metas).toHaveLength(1);
                expect(count(built(), 'name="capsulebin-languages"')).toBe(1);
                const meta = metas[0]![0];
                expect(meta).toContain(`content="${KNOWN_LANGUAGES.join(' ')}"`);
                expect(meta).toContain('data-default="%%capsulebin:default%%"');
                expect(head().indexOf(meta)).toBeLessThan(head().indexOf('/language-init.js'));
            });

            it('declares the color scheme once, before every script', () => {
                const metas = [...head().matchAll(/<meta name="color-scheme"[^>]*>/g)];
                expect(metas).toHaveLength(1);
                expect(metas[0]![0]).toContain('content="light dark"');
                expect(metas[0]!.index).toBeLessThan(head().indexOf('<script'));
            });

            it('loads theme-init.js, then language-init.js blocking, then every module', () => {
                const scripts = [...head().matchAll(/<script\b[^>]*>/g)].map(([tag]) => tag);
                const theme = scripts.findIndex((tag) => tag.includes('src="/theme-init.js"'));
                const init = scripts.findIndex((tag) => tag.includes('src="/language-init.js"'));
                const firstModule = scripts.findIndex((tag) => tag.includes('type="module"'));
                expect(theme).toBe(0);
                expect(init).toBe(1);
                expect(scripts[theme]).not.toMatch(/\b(defer|async|type)\b/);
                expect(scripts[init]).not.toMatch(/\b(defer|async|type)\b/);
                expect(firstModule).toBeGreaterThan(init);
                // No script outside the head could run before it either.
                expect(count(built(), '<script')).toBe(scripts.length);
            });

            it('loads its catalogue module before the page script, everything from /assets/', () => {
                const modules = [
                    ...head().matchAll(/<script type="module"[^>]*\bsrc="([^"]+)"/g),
                ].map(([, src = '']) => src);
                const register = modules.findIndex((src) =>
                    src.startsWith(`/assets/register-${lang}-`),
                );
                expect(register, modules.join(' ')).toBeGreaterThanOrEqual(0);
                // The page script is the last module of the shell.
                expect(register).toBeLessThan(modules.length - 1);
                for (const other of KNOWN_LANGUAGES.filter((known) => known !== lang)) {
                    expect(built()).not.toContain(`register-${other}-`);
                }

                const loaded = [
                    ...built().matchAll(/<(?:script|link)\b[^>]*\b(?:src|href)="([^"]+)"/g),
                ]
                    .map(([, url = '']) => url)
                    .filter((url) => !['/language-init.js', '/theme-init.js'].includes(url));
                expect(loaded.length).toBeGreaterThan(0);
                expect(loaded.filter((url) => !url.startsWith('/assets/'))).toEqual([]);
            });

            it('starts every internal link with the prefix token and carries no other token', () => {
                const html = built();
                const links = [...html.matchAll(/<a\b[^>]*\bhref="([^"]*)"/g)].map(
                    ([, href = '']) => href,
                );
                expect(links.length).toBeGreaterThan(0);
                for (const href of links) {
                    expect(href.startsWith('%%capsulebin:prefix%%/'), href).toBe(true);
                    expect(href, href).not.toMatch(
                        new RegExp(`^%%capsulebin:prefix%%/(${KNOWN_LANGUAGES.join('|')})(/|$)`),
                    );
                }
                const rest = html
                    .replaceAll('href="%%capsulebin:prefix%%/', '')
                    .replaceAll('data-default="%%capsulebin:default%%"', '');
                expect(rest).not.toContain('%%');
            });
        },
    );
});
