// @vitest-environment node
//
// Works on build outputs written to a temporary directory and needs no DOM.

/**
 * scripts/check-dist.ts, which `npm run build` runs on the real output: here
 * against small hand-made outputs, so that each rule is seen to catch what it
 * is for.
 */

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import { type DistInput, checkDist } from '../scripts/check-dist';

const INPUT: DistInput = {
    catalogues: {
        en: { 'view.copy': 'Copy text', 'view.title': 'Capsule', 'page.heading': 'Create one' },
        de: { 'view.copy': 'Text kopieren', 'view.title': 'Capsule', 'page.heading': 'Erstellen' },
    },
    templateOnlyKeys: ['page.heading'],
};

/** A language-free library chunk, used by both languages. */
const VENDOR = 'export const html = (s) => s;';

type Assets = Record<string, string>;

/** The directories `dist()` created, removed after each test. */
const created: string[] = [];

afterEach(() => {
    for (const dir of created.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/** Writes `dist/` with one page per language, loading `entry-<lang>.js`. */
function dist(assets: Assets): string {
    const dir = mkdtempSync(join(tmpdir(), 'capsulebin-dist-'));
    created.push(dir);
    mkdirSync(join(dir, 'assets'));
    for (const [name, code] of Object.entries(assets))
        writeFileSync(join(dir, 'assets', name), code);
    for (const lang of ['en', 'de']) {
        mkdirSync(join(dir, lang));
        writeFileSync(
            join(dir, lang, 'p.html'),
            `<html lang="${lang}"><script type="module" src="/assets/entry-${lang}.js"></script></html>`,
        );
    }
    return dir;
}

const GOOD: Assets = {
    'entry-en.js': 'import"./vendor-a.js";import"./texts-en.js";',
    'entry-de.js': 'import"./vendor-a.js";import"./texts-de.js";',
    'texts-en.js': 'export default {"view.copy":"Copy text","view.title":`Capsule`};',
    'texts-de.js': 'export default {"view.copy":"Text kopieren","view.title":`Capsule`};',
    'vendor-a.js': VENDOR,
};

describe('build output', () => {
    it('passes with one language per page and the libraries once', () => {
        expect(checkDist(dist(GOOD), INPUT)).toEqual([]);
    });

    it('fails without a build', () => {
        expect(checkDist(join(tmpdir(), 'capsulebin-no-such-dist'), INPUT)).toEqual([
            expect.stringMatching(/assets does not exist/),
        ]);
    });

    it('finds a text of the other language, also in an imported chunk', () => {
        const problems = checkDist(
            dist({ ...GOOD, 'vendor-a.js': `${VENDOR}const x="Text kopieren";` }),
            INPUT,
        );
        expect(problems).toContain('en: vendor-a.js contains the de text "Text kopieren"');
    });

    it('follows imports of chunks whose names carry any characters', () => {
        const problems = checkDist(
            dist({
                ...GOOD,
                'vendor-a.js': `${VENDOR}import"./vendor~x-B_1.js";`,
                'vendor~x-B_1.js': 'const x="Text kopieren";',
            }),
            INPUT,
        );
        expect(problems).toContain('en: vendor~x-B_1.js contains the de text "Text kopieren"');
    });

    it('ignores a word that is only part of an identifier or a longer string', () => {
        const code = `${VENDOR}const CopyText="Copy text twice";`;
        expect(checkDist(dist({ ...GOOD, 'vendor-a.js': code }), INPUT)).toEqual([]);
    });

    it('finds a template-only key in any chunk', () => {
        const problems = checkDist(
            dist({ ...GOOD, 'texts-de.js': 'export default {"page.heading":"Erstellen"};' }),
            INPUT,
        );
        expect(problems).toContain('texts-de.js names the template-only key page.heading');
    });

    it('finds a chunk without texts that was built once per language', () => {
        const problems = checkDist(
            dist({
                ...GOOD,
                'entry-de.js': 'import"./vendor-b.js";import"./texts-de.js";',
                'vendor-b.js': VENDOR,
            }),
            INPUT,
        );
        expect(problems).toContainEqual(expect.stringMatching(/chunks without texts differ/));
        expect(problems).toContainEqual(
            expect.stringMatching(/vendor-[ab]\.js and vendor-[ab]\.js are the same chunk/),
        );
    });
});
