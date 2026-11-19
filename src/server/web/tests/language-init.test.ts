/**
 * `public/language-init.js`: which language it picks, when it moves the page, and
 * that it leaves the page alone whenever it cannot decide safely.
 *
 * The script runs with `document`, `location`, `localStorage` and `navigator`
 * passed in as parameters, which shadow the globals of the same name, so every
 * case controls exactly what the script sees.
 */

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

import { LANGUAGE_STORAGE_KEY } from '../src/language';
import { SHELLS, readTemplate } from '../scripts/shells';

const source = readFileSync(resolve(__dirname, '../public/language-init.js'), 'utf8');

interface Page {
    path: string;
    search?: string;
    hash?: string;
    /** `content` and `data-default` of the meta element; `null` for no meta. */
    meta?: { content: string; default: string } | null;
    /** Stored choice; `'throws'` for a blocked localStorage. */
    stored?: string | null | 'throws';
    browser?: string[];
}

/** Runs the script for `page`; returns the target of `location.replace`, if any. */
function run(page: Page): string | null {
    const doc = document.implementation.createHTMLDocument();
    const meta = page.meta === undefined ? { content: 'en de', default: 'en' } : page.meta;
    if (meta) {
        const el = doc.createElement('meta');
        el.name = 'capsulebin-languages';
        el.content = meta.content;
        el.dataset.default = meta.default;
        doc.head.append(el);
    }

    let replaced: string | null = null;
    const location = {
        pathname: page.path,
        search: page.search ?? '',
        hash: page.hash ?? '',
        replace: (url: string) => {
            replaced = url;
        },
    };
    const storage = {
        getItem: (key: string) => {
            if (page.stored === 'throws') throw new DOMException('blocked', 'SecurityError');
            return key === LANGUAGE_STORAGE_KEY ? (page.stored ?? null) : null;
        },
    };
    const navigator = { languages: page.browser ?? [] };

    new Function('document', 'location', 'localStorage', 'navigator', source)(
        doc,
        location,
        storage,
        navigator,
    );
    return replaced;
}

describe('language-init.js', () => {
    it('uses the same storage key as language.ts', () => {
        expect(source).toContain(`'${LANGUAGE_STORAGE_KEY}'`);
    });

    it('is not a module and loads nothing', () => {
        expect(source).not.toMatch(/\bimport\b|\bexport\b|\bfetch\b/);
    });

    it.each(SHELLS)('is loaded by %s.html blocking, before theme-init.js', (shell) => {
        const head = readTemplate(shell).split('</head>')[0]!;
        expect(head).not.toMatch(/<script[^>]*language-init\.js[^>]*(defer|async|type=)/);
        expect(head.indexOf('/language-init.js')).toBeLessThan(head.indexOf('/theme-init.js'));
    });

    it('prefers the stored choice over the browser language', () => {
        expect(run({ path: '/en/', stored: 'de', browser: ['en-US'] })).toBe('/de/');
    });

    it('ignores a stored language that is not offered', () => {
        expect(
            run({ path: '/de/', stored: 'de', meta: { content: 'en', default: 'en' } }),
        ).toBeNull();
        expect(run({ path: '/en/', stored: 'fr', browser: ['de-AT'] })).toBe('/de/');
    });

    it('takes the first browser language whose primary subtag is offered', () => {
        expect(run({ path: '/en/report', browser: ['fr-FR', 'de-CH', 'en'] })).toBe('/de/report');
        expect(run({ path: '/de/report', browser: ['EN-gb', 'de'] })).toBe('/en/report');
    });

    it("falls back to the operator's default", () => {
        expect(
            run({ path: '/de/', browser: ['fr'], meta: { content: 'en de', default: 'en' } }),
        ).toBe('/en/');
        expect(run({ path: '/en/', browser: [], meta: { content: 'en de', default: 'de' } })).toBe(
            '/de/',
        );
    });

    it('keeps the rest of the path, the query and the fragment unchanged', () => {
        expect(
            run({
                path: '/en/p/01JABCDEFGHJKMNPQRSTVWXYZ0',
                search: '?x=1',
                hash: '#v1.c2VjcmV0',
                stored: 'de',
            }),
        ).toBe('/de/p/01JABCDEFGHJKMNPQRSTVWXYZ0?x=1#v1.c2VjcmV0');
    });

    it('stays when the page already has the chosen language', () => {
        expect(run({ path: '/de/manage/x', stored: 'de', browser: ['en'] })).toBeNull();
        expect(run({ path: '/en/', browser: ['en-US'] })).toBeNull();
    });

    it('does nothing without the meta element', () => {
        expect(run({ path: '/en/', stored: 'de', meta: null })).toBeNull();
    });

    it('does nothing without localStorage', () => {
        expect(run({ path: '/en/', stored: 'throws', browser: ['de'] })).toBeNull();
    });

    it('does nothing on a path without an offered prefix', () => {
        expect(run({ path: '/fr/', stored: 'de' })).toBeNull();
        expect(run({ path: '/', stored: 'de' })).toBeNull();
    });
});
