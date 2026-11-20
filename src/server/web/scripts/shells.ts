/**
 * Generates one HTML shell per interface language from the templates in the
 * web root and the catalogues in `src/i18n/`.
 *
 * Templates are ordinary HTML with these placeholders:
 *
 * - `{{lang}}` - the language code, for `<html lang>`;
 * - `{{languages}}` - every known language, space-separated, for the
 *   `<meta name="capsulebin-languages">` that `public/language-init.js` and the
 *   language switch read;
 * - `{{prefix}}` and `{{default}}` - the prefix of internal links
 *   (`href="{{prefix}}/report"`) and the meta's `data-default`. Both depend on the
 *   instance's default language, which is configuration and so unknown here: the
 *   generator writes the tokens {@link PREFIX_TOKEN} and {@link DEFAULT_TOKEN},
 *   which pass through the build unchanged. The server replaces them at startup
 *   (no prefix for the default language, `/<lang>` for every other), the Vite dev
 *   server per request ({@link fillServerTokens});
 * - `{{<catalogue key>}}` - a text from the catalogue. Catalogue values are plain
 *   text and are HTML-escaped here, so a catalogue cannot bring markup or script
 *   into a page.
 *
 * A catalogue text may also sit inside one of the server's marker comments: the
 * footer's legal links are `<!--capsulebin:legal-link:<page>:{{shell.<page>}}-->`,
 * which the server turns into a link or drops. Escaping leaves no `>`, so the text
 * cannot end the comment; but it does not touch `-`, and the server ends the label
 * at the first `-->`. So a text inside a comment must not contain `--` or end in
 * `-` - the build fails otherwise.
 *
 * The output is a pure function of templates and catalogues: no timestamps, files
 * in sorted order. That keeps the web build reproducible
 * (`scripts/reproducible-web.sh`).
 */

import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

// Deep import: plain Node (`npm run build`) cannot resolve core's extensionless index imports.
import { KNOWN_LANGUAGES } from '@capsulebin/core/src/languages.ts';

import { languagePrefix } from '../src/language-prefix.ts';

const WEB_ROOT = resolve(import.meta.dirname, '..');

/** The shell templates, by file name, in the web root. */
export const SHELLS = ['index', 'p', 'manage', 'report', 'legal'] as const;

export type ShellName = (typeof SHELLS)[number];

/**
 * Where the generated shells go during a build or the dev server, relative to the
 * web root (Vite's root). `vite.config.ts` moves them to `dist/<lang>/` in the
 * output. Ignored by git.
 */
export const GENERATED_DIR = '.shells';

const CATALOGUE_DIR = join(WEB_ROOT, 'src', 'i18n');

/** Token for the prefix of internal links: empty for the default language, else `/<lang>`. */
export const PREFIX_TOKEN = '%%capsulebin:prefix%%';

/** Token for the instance's default language, in the languages meta. */
export const DEFAULT_TOKEN = '%%capsulebin:default%%';

/**
 * Placeholders filled from the language rather than the catalogue; catalogue keys
 * always contain a dot, so they cannot clash.
 */
function languagePlaceholders(lang: string): ReadonlyMap<string, string> {
    return new Map([
        ['lang', lang],
        ['languages', KNOWN_LANGUAGES.join(' ')],
        ['prefix', PREFIX_TOKEN],
        ['default', DEFAULT_TOKEN],
    ]);
}

/**
 * Replaces the tokens in a generated shell of language `lang` the way the server
 * does for an instance whose default language is `defaultLanguage`.
 */
export function fillServerTokens(html: string, lang: string, defaultLanguage: string): string {
    return html
        .replaceAll(PREFIX_TOKEN, languagePrefix(defaultLanguage, lang))
        .replaceAll(DEFAULT_TOKEN, defaultLanguage);
}

const PLACEHOLDER = /\{\{([A-Za-z0-9_.]+)\}\}/g;

export type Catalogue = Readonly<Record<string, string>>;

/** The interface languages: one per catalogue file in `src/i18n/`, sorted. */
export function catalogueLanguages(): string[] {
    return readdirSync(CATALOGUE_DIR)
        .filter((name) => name.endsWith('.json'))
        .map((name) => name.slice(0, -'.json'.length))
        .sort();
}

export function readCatalogue(lang: string): Catalogue {
    return JSON.parse(readFileSync(join(CATALOGUE_DIR, `${lang}.json`), 'utf8')) as Catalogue;
}

export function readTemplate(shell: ShellName): string {
    return readFileSync(join(WEB_ROOT, `${shell}.html`), 'utf8');
}

function escapeHtml(text: string): string {
    return text
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&#39;');
}

/** Whether `offset` in `html` lies inside an HTML comment. */
function insideComment(html: string, offset: number): boolean {
    return html.lastIndexOf('<!--', offset) > html.lastIndexOf('-->', offset);
}

/**
 * Whether `text` can sit inside a marker comment unchanged: with `--` or a trailing
 * `-` it could end the comment early or be cut short by the server's `-->` search.
 */
export function fitsInComment(text: string): boolean {
    return !text.includes('--') && !text.endsWith('-');
}

/**
 * Fills a template for one language. A placeholder without a catalogue entry is
 * an error rather than an empty string: a missing text must fail the build, not
 * ship as a blank button. So is a text inside a comment that does not fit there.
 */
export function renderShell(template: string, lang: string, catalogue: Catalogue): string {
    const fromLanguage = languagePlaceholders(lang);
    return template.replace(PLACEHOLDER, (placeholder, key: string, offset: number) => {
        const value = fromLanguage.get(key) ?? catalogue[key];
        if (value === undefined) {
            throw new Error(`${lang}.json has no entry for ${placeholder}`);
        }
        if (insideComment(template, offset) && !fitsInComment(value)) {
            throw new Error(
                `${lang}.json: ${placeholder} sits inside a comment and must not contain "--" or end in "-"`,
            );
        }
        return escapeHtml(value);
    });
}

/** The catalogue keys whose placeholders sit inside a comment in `template`. */
export function keysInComments(template: string): string[] {
    return [...template.matchAll(PLACEHOLDER)]
        .filter((match) => insideComment(template, match.index))
        .map((match) => match[1]!);
}

/** The shell `shell` in language `lang`, as the build produces it (before Vite). */
export function shellFor(shell: ShellName, lang: string): string {
    return renderShell(readTemplate(shell), lang, readCatalogue(lang));
}

/** Path of a generated shell relative to the web root, e.g. `.shells/de/p.html`. */
export function generatedPath(lang: string, shell: ShellName): string {
    return `${GENERATED_DIR}/${lang}/${shell}.html`;
}

/**
 * Writes every shell in every language below `GENERATED_DIR`, replacing what was
 * there, and returns the written paths relative to the web root.
 */
export function generateShells(): string[] {
    const outDir = join(WEB_ROOT, GENERATED_DIR);
    rmSync(outDir, { recursive: true, force: true });

    const written: string[] = [];
    for (const lang of catalogueLanguages()) {
        const catalogue = readCatalogue(lang);
        mkdirSync(join(outDir, lang), { recursive: true });
        for (const shell of SHELLS) {
            const path = generatedPath(lang, shell);
            writeFileSync(join(WEB_ROOT, path), renderShell(readTemplate(shell), lang, catalogue));
            written.push(path);
        }
    }
    return written;
}
