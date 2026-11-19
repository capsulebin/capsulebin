/**
 * Generates one HTML shell per interface language from the templates in the
 * web root and the catalogues in `src/i18n/`.
 *
 * Templates are ordinary HTML with two kinds of placeholders:
 *
 * - `{{lang}}` - the language code, for `<html lang>` and the prefix of internal
 *   links (`href="/{{lang}}/report"`);
 * - `{{<catalogue key>}}` - a text from the catalogue. Catalogue values are plain
 *   text and are HTML-escaped here, so a catalogue cannot bring markup or script
 *   into a page.
 *
 * The output is a pure function of templates and catalogues: no timestamps, files
 * in sorted order. That keeps the web build reproducible
 * (`scripts/reproducible-web.sh`).
 */

import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

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

/** Language-code placeholder; catalogue keys always contain a dot, so it cannot clash. */
const LANG_PLACEHOLDER = 'lang';

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

/**
 * Fills a template for one language. A placeholder without a catalogue entry is
 * an error rather than an empty string: a missing text must fail the build, not
 * ship as a blank button.
 */
export function renderShell(template: string, lang: string, catalogue: Catalogue): string {
    return template.replace(PLACEHOLDER, (_, key: string) => {
        if (key === LANG_PLACEHOLDER) return lang;
        const value = catalogue[key];
        if (value === undefined) {
            throw new Error(`${lang}.json has no entry for {{${key}}}`);
        }
        return escapeHtml(value);
    });
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
