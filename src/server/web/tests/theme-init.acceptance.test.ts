/**
 * AC29 edges of `public/theme-init.js`, derived from the criterion: narrowing the
 * color-scheme meta must never cost the theme itself, and the script must touch
 * only the meta it is meant for.
 */

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const initJs = readFileSync(resolve(__dirname, '../public/theme-init.js'), 'utf8');

function storageWith(choice: string): Pick<Storage, 'getItem'> {
    return { getItem: (key: string) => (key === 'capsulebin.theme' ? choice : null) };
}

function run(doc: Document, choice: string): void {
    new Function('document', 'localStorage', initJs)(doc, storageWith(choice));
}

describe('theme-init.js (AC29 edges)', () => {
    it.each(['light', 'dark'])(
        'still sets data-theme for %s on a document without the color-scheme meta',
        (choice) => {
            const doc = document.implementation.createHTMLDocument();
            expect(() => run(doc, choice)).not.toThrow();
            expect(doc.documentElement.getAttribute('data-theme')).toBe(choice);
        },
    );

    it('changes only the color-scheme meta, no other meta in the head', () => {
        const doc = document.implementation.createHTMLDocument();
        const scheme = doc.createElement('meta');
        scheme.name = 'color-scheme';
        scheme.content = 'light dark';
        const languages = doc.createElement('meta');
        languages.name = 'capsulebin-languages';
        languages.content = 'en de';
        languages.setAttribute('data-default', 'en');
        doc.head.append(languages, scheme);

        run(doc, 'dark');

        expect(scheme.getAttribute('content')).toBe('dark');
        expect(languages.getAttribute('content')).toBe('en de');
        expect(languages.getAttribute('data-default')).toBe('en');
    });
});
