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
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const STORAGE_KEY = 'capsulebin.theme';

const initJs = readFileSync(resolve(__dirname, '../public/theme-init.js'), 'utf8');

const themeTs = readFileSync(resolve(__dirname, '../src/theme.ts'), 'utf8');

/**
 * Runs the script against a fresh document whose head carries the shells'
 * color-scheme meta. `document` and `localStorage` are passed in as parameters,
 * shadowing the globals, so every case controls what the script sees.
 * `stored: 'throws'` stands for a blocked localStorage.
 */
function run(stored: string | null | 'throws'): { theme: string | null; scheme: string | null } {
    const doc = document.implementation.createHTMLDocument();
    const meta = doc.createElement('meta');
    meta.name = 'color-scheme';
    meta.content = 'light dark';
    doc.head.append(meta);
    const storage = {
        getItem: (key: string) => {
            if (stored === 'throws') throw new DOMException('blocked', 'SecurityError');
            return key === STORAGE_KEY ? stored : null;
        },
    };

    new Function('document', 'localStorage', initJs)(doc, storage);
    return {
        theme: doc.documentElement.getAttribute('data-theme'),
        scheme: meta.getAttribute('content'),
    };
}

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

    /**
     * The color-scheme meta gives the never-styled document the theme's canvas when
     * language-init.js moves to another language (no bright flash in dark mode).
     */
    it.each(['light', 'dark'])(
        'applies a stored %s choice to data-theme and the meta',
        (choice) => {
            expect(run(choice)).toEqual({ theme: choice, scheme: choice });
        },
    );

    it.each([null, 'auto', 'sepia', 'throws'])(
        'leaves data-theme and the meta alone for the stored value %s',
        (stored) => {
            expect(run(stored)).toEqual({ theme: null, scheme: 'light dark' });
        },
    );

    /** Without try/catch a blocked localStorage would stall the whole page. */
    it('handles a blocked localStorage', () => {
        expect(initJs).toMatch(/try\s*\{[\s\S]*catch/);
    });
});

describe('HTML shells', () => {
    const shells = ['index.html', 'p.html', 'manage.html', 'report.html', 'legal.html'] as const;

    function shell(name: string): string {
        return readFileSync(resolve(__dirname, `../${name}`), 'utf8');
    }

    it.each(shells)('%s loads theme-init.js blocking in the head', (name) => {
        const html = shell(name);
        const head = html.slice(0, html.indexOf('</head>'));

        expect(head).toContain('<script src="/theme-init.js"></script>');
        // Neither defer nor async nor type=module - any of those would run too late.
        expect(head).not.toMatch(/<script[^>]*theme-init\.js[^>]*(defer|async|type=)/);
    });

    /** The meta must already be parsed when the head scripts run. */
    it.each(shells)('%s declares the color scheme before both head scripts', (name) => {
        const html = shell(name);
        const head = html.slice(0, html.indexOf('</head>'));
        const meta = head.indexOf('<meta name="color-scheme" content="light dark" />');

        expect(meta).toBeGreaterThanOrEqual(0);
        expect(head.match(/name="color-scheme"/g)).toHaveLength(1);
        expect(meta).toBeLessThan(head.indexOf('<script'));
        expect(head.indexOf('/theme-init.js')).toBeLessThan(head.indexOf('/language-init.js'));
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
