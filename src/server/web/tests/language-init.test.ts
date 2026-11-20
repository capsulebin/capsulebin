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

    it.each(SHELLS)('is loaded by %s.html blocking, after theme-init.js', (shell) => {
        const head = readTemplate(shell).split('</head>')[0]!;
        expect(head).not.toMatch(/<script[^>]*language-init\.js[^>]*(defer|async|type=)/);
        expect(head.indexOf('/theme-init.js')).toBeLessThan(head.indexOf('/language-init.js'));
    });

    it('prefers the stored choice over the browser language', () => {
        expect(run({ path: '/', stored: 'de', browser: ['en-US'] })).toBe('/de/');
        expect(run({ path: '/de/', stored: 'en', browser: ['de-DE'] })).toBe('/');
    });

    it('ignores a stored language that is not offered', () => {
        expect(
            run({ path: '/de/', stored: 'de', meta: { content: 'en', default: 'en' } }),
        ).toBeNull();
        expect(run({ path: '/', stored: 'fr', browser: ['de-AT'] })).toBe('/de/');
    });

    it('takes the first browser language whose primary subtag is offered', () => {
        expect(run({ path: '/report', browser: ['fr-FR', 'de-CH', 'en'] })).toBe('/de/report');
        expect(run({ path: '/de/report', browser: ['EN-gb', 'de'] })).toBe('/report');
    });

    it('falls back to the default language (data-default)', () => {
        expect(run({ path: '/de/', browser: ['fr'] })).toBe('/');
        expect(run({ path: '/en/', browser: [], meta: { content: 'en de', default: 'de' } })).toBe(
            '/',
        );
    });

    it('treats a path without a known prefix as the default language', () => {
        expect(run({ path: '/p/abc', browser: ['en-US'] })).toBeNull();
        expect(run({ path: '/p/abc', browser: ['de-DE'] })).toBe('/de/p/abc');
        expect(run({ path: '/fr/', stored: 'de' })).toBe('/de/fr/');
    });

    it('follows the configured default language', () => {
        const meta = { content: 'en de', default: 'de' };
        expect(run({ path: '/p/abc', browser: ['de-DE'], meta })).toBeNull();
        expect(run({ path: '/p/abc', browser: ['en-US'], meta })).toBe('/en/p/abc');
        expect(run({ path: '/en/p/abc', browser: ['en-US'], meta })).toBeNull();
        expect(run({ path: '/en/p/abc', stored: 'de', browser: ['en-US'], meta })).toBe('/p/abc');
    });

    it('keeps the rest of the path, the query and the fragment unchanged', () => {
        const page = { search: '?x=1', hash: '#v1.c2VjcmV0' };
        expect(run({ ...page, path: '/p/01JABCDEFGHJKMNPQRSTVWXYZ0', stored: 'de' })).toBe(
            '/de/p/01JABCDEFGHJKMNPQRSTVWXYZ0?x=1#v1.c2VjcmV0',
        );
        expect(run({ ...page, path: '/de/p/01JABCDEFGHJKMNPQRSTVWXYZ0', stored: 'en' })).toBe(
            '/p/01JABCDEFGHJKMNPQRSTVWXYZ0?x=1#v1.c2VjcmV0',
        );
    });

    it('leads from the bare prefix to the start page', () => {
        expect(run({ path: '/de', stored: 'en' })).toBe('/');
    });

    it('stays when the page already has the chosen language', () => {
        expect(run({ path: '/de/manage/x', stored: 'de', browser: ['en'] })).toBeNull();
        expect(run({ path: '/', browser: ['en-US'] })).toBeNull();
        expect(run({ path: '/manage/x', stored: 'en', browser: ['de'] })).toBeNull();
    });

    it('does nothing without the meta element', () => {
        expect(run({ path: '/', stored: 'de', meta: null })).toBeNull();
    });

    it.each(['%%capsulebin:default%%', 'fr', ''])(
        'does nothing without a usable default language: %j',
        (fallback) => {
            const meta = { content: 'en de', default: fallback };
            expect(run({ path: '/', stored: 'de', meta })).toBeNull();
            expect(run({ path: '/de/', stored: 'en', meta })).toBeNull();
        },
    );

    it('does nothing without localStorage', () => {
        expect(run({ path: '/', stored: 'throws', browser: ['de'] })).toBeNull();
    });
});
