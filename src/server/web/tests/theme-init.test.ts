// @vitest-environment node
//
// Inspects public/theme-init.js as a file. Kept separate from theme.test.ts,
// where jsdom runs and `import.meta.url` is not a file: URL.

/**
 * `public/theme-init.js` is the only place allowed to duplicate the storage key.
 * The reason is stated in the file itself: importing it would turn it into a
 * module and therefore defer it - and deferred is exactly what it must not be,
 * because it has to run before the first paint.
 *
 * These tests keep the duplication in sync and pin down the properties the file
 * exists for.
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const STORAGE_KEY = 'capsulebin.theme';

const initJs = readFileSync(
    fileURLToPath(new URL('../public/theme-init.js', import.meta.url)),
    'utf8',
);

const themeTs = readFileSync(fileURLToPath(new URL('../src/theme.ts', import.meta.url)), 'utf8');

describe('theme-init.js', () => {
    it('uses the same storage key as theme.ts', () => {
        expect(initJs).toContain(`'${STORAGE_KEY}'`);
        expect(themeTs).toContain(`'${STORAGE_KEY}'`);
    });

    /**
     * An `import`/`export` would make the file a module (deferred); a `fetch` would
     * either violate the CSP or delay startup.
     */
    it('is not a module and loads nothing', () => {
        expect(initJs).not.toMatch(/\bimport\b|\bexport\b|\bfetch\b/);
    });

    /** For 'auto' the attribute must stay unset, otherwise the media query is bypassed. */
    it('sets data-theme only for an explicit choice', () => {
        expect(initJs).toContain("choice === 'light' || choice === 'dark'");
    });

    /** Without try/catch a blocked localStorage would stall the whole page. */
    it('handles a blocked localStorage', () => {
        expect(initJs).toMatch(/try\s*\{[\s\S]*catch/);
    });
});

describe('HTML shells', () => {
    const shells = ['index.html', 'p.html', 'manage.html', 'report.html', 'legal.html'] as const;

    function shell(name: string): string {
        return readFileSync(fileURLToPath(new URL(`../${name}`, import.meta.url)), 'utf8');
    }

    it.each(shells)('%s loads theme-init.js blocking in the head', (name) => {
        const html = shell(name);
        const head = html.slice(0, html.indexOf('</head>'));

        expect(head).toContain('<script src="/theme-init.js"></script>');
        // Neither defer nor async nor type=module - any of those would run too late.
        expect(head).not.toMatch(/<script[^>]*theme-init\.js[^>]*(defer|async|type=)/);
    });

    /**
     * The CSP forbids inline scripts and inline styles. A violation in the shipped
     * HTML would silently break the page in the browser, so it surfaces here rather
     * than in Playwright.
     */
    it.each(shells)('%s contains no inline script and no style attribute', (name) => {
        const html = shell(name);

        expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>[\s\S]*?\S[\s\S]*?<\/script>/);
        expect(html).not.toMatch(/\sstyle="/);
        expect(html).not.toMatch(/<style[\s>]/);
        // No third party resources: everything comes from the binary.
        expect(html).not.toMatch(/(src|href)="https?:\/\//);
    });
});
