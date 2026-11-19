/**
 * The shell templates and the per-language shells generated from them
 * (scripts/shells.ts): every visible text comes from the catalogue, nothing of
 * the template machinery is left over, the server's markers are all there, the
 * languages meta is written at build time, and every internal link stays within
 * the page's language.
 */

import { KNOWN_LANGUAGES } from '@capsulebin/core';
import { describe, expect, it } from 'vitest';

import {
    SHELLS,
    catalogueLanguages,
    fitsInComment,
    keysInComments,
    readCatalogue,
    readTemplate,
    renderShell,
    shellFor,
} from '../scripts/shells';

const LANGUAGES = catalogueLanguages();

const CASES = LANGUAGES.flatMap((lang) => SHELLS.map((shell) => [lang, shell] as const));

/**
 * Literals that may stay in a template: the product name, symbols, and the names
 * of the highlight.js languages in the code-language menu.
 */
const ALLOWED_LITERALS = new Set([
    'CapsuleBin',
    '· CapsuleBin',
    '◐',
    '#',
    'Bash',
    'CSS',
    'Diff',
    'HTML',
    'INI / TOML',
    'JavaScript',
    'JSON',
    'Markdown',
    'Python',
    'Rust',
    'SQL',
    'TypeScript',
    'YAML',
]);

/** The server's markers and how often each shell carries them. */
const MARKERS = {
    '<!--capsulebin:legal-title-->': (shell: string) => (shell === 'legal' ? 2 : 0),
    '<!--capsulebin:legal-content-->': (shell: string) => (shell === 'legal' ? 1 : 0),
};

const TEXT_ATTRIBUTES = ['title', 'aria-label', 'placeholder', 'alt'];

function parse(html: string): Document {
    return new DOMParser().parseFromString(html, 'text/html');
}

/** Text nodes and text attributes of `root`, including the content of <template>s. */
function visibleTexts(root: Document | DocumentFragment | Element): string[] {
    const texts: string[] = [];
    const visit = (node: Node): void => {
        if (node.nodeType === Node.TEXT_NODE) {
            texts.push(node.textContent ?? '');
        } else if (node instanceof Element) {
            for (const name of TEXT_ATTRIBUTES) {
                const value = node.getAttribute(name);
                if (value !== null) texts.push(value);
            }
            if (node instanceof HTMLTemplateElement) visit(node.content);
        }
        if (!(node instanceof Element && ['SCRIPT', 'STYLE'].includes(node.tagName))) {
            node.childNodes.forEach(visit);
        }
    };
    visit(root);
    return texts;
}

describe('templates', () => {
    it.each(SHELLS)('%s.html takes every visible text from the catalogue', (shell) => {
        const leftovers = visibleTexts(parse(readTemplate(shell)))
            .map((text) =>
                text
                    .replace(/\{\{[A-Za-z0-9_.]+\}\}/g, ' ')
                    .replace(/<!--capsulebin:[a-z-]+-->/g, ' ')
                    .replace(/\s+/g, ' ')
                    .trim(),
            )
            .filter((text) => text !== '' && !ALLOWED_LITERALS.has(text));

        expect(leftovers).toEqual([]);
    });

    it.each(SHELLS)('%s.html loads the head scripts in order, blocking', (shell) => {
        const html = readTemplate(shell);
        const head = html.slice(0, html.indexOf('</head>'));
        const order = [
            'name="capsulebin-languages"',
            '<script src="/language-init.js"></script>',
            '<script src="/theme-init.js"></script>',
            '<script type="module"',
        ].map((part) => head.indexOf(part));

        expect(order.every((index) => index >= 0)).toBe(true);
        expect([...order].sort((a, b) => a - b)).toEqual(order);
    });
});

describe('generator', () => {
    it('escapes catalogue values', () => {
        const html = renderShell('<p title="{{a.b}}">{{a.b}}</p>', 'en', {
            'a.b': `<img src=x onerror="alert('1')"> & co`,
        });
        expect(html).toBe(
            '<p title="&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt; &amp; co">' +
                '&lt;img src=x onerror=&quot;alert(&#39;1&#39;)&quot;&gt; &amp; co</p>',
        );
    });

    it('keeps a catalogue value inside a marker comment', () => {
        const html = renderShell('<!--capsulebin:legal-link:terms:{{a.b}}-->', 'en', {
            'a.b': '><script>alert(1)</script>',
        });
        expect(html).toBe(
            '<!--capsulebin:legal-link:terms:&gt;&lt;script&gt;alert(1)&lt;/script&gt;-->',
        );
    });

    /** Escaping leaves `-` alone; the server ends the label at the first `-->`. */
    it.each(['Terms-', 'A--B', '--', '-', 'x--><b>'])(
        'fails on a value inside a comment that contains "--" or ends in "-": %j',
        (value) => {
            expect(() =>
                renderShell('<!--capsulebin:legal-link:terms:{{a.b}}-->', 'de', { 'a.b': value }),
            ).toThrow(/de\.json.*\{\{a\.b\}\}.*comment/);
        },
    );

    it('allows such a value outside a comment', () => {
        expect(renderShell('<!-- x --><p>{{a.b}}</p>', 'en', { 'a.b': 'A--B-' })).toBe(
            '<!-- x --><p>A--B-</p>',
        );
    });

    it('allows a single inner hyphen inside a comment', () => {
        expect(
            renderShell('<!--capsulebin:legal-link:terms:{{a.b}}-->', 'en', { 'a.b': 'Opt-in' }),
        ).toBe('<!--capsulebin:legal-link:terms:Opt-in-->');
    });

    it('fails on a text the catalogue does not have', () => {
        expect(() => renderShell('{{missing.key}}', 'de', {})).toThrow(/de\.json.*missing\.key/);
    });
});

describe.each(LANGUAGES)('catalogue %s', (lang) => {
    const catalogue = readCatalogue(lang);
    const keys = [...new Set(SHELLS.flatMap((shell) => keysInComments(readTemplate(shell))))];

    it('has the footer labels among the texts placed inside comments', () => {
        expect(keys).toEqual(
            expect.arrayContaining(['shell.imprint', 'shell.privacy', 'shell.terms']),
        );
    });

    it.each(keys)('keeps %s fit for a comment: no "--", no trailing "-"', (key) => {
        expect(fitsInComment(catalogue[key]!)).toBe(true);
    });
});

describe.each(CASES)('generated %s/%s.html', (lang, shell) => {
    const html = shellFor(shell, lang);
    const doc = parse(html);

    /** Screen readers pick the pronunciation from it, so it must match the texts. */
    it('declares its language', () => {
        expect(doc.documentElement.getAttribute('lang')).toBe(lang);
    });

    it('has no placeholder and no catalogue key left', () => {
        expect(html).not.toMatch(/\{\{|\}\}/);
        for (const key of Object.keys(readCatalogue(lang))) {
            expect(html).not.toContain(key);
        }
    });

    it("carries the server's markers as often as before", () => {
        for (const [marker, count] of Object.entries(MARKERS)) {
            expect(html.split(marker).length - 1, marker).toBe(count(shell));
        }
        expect(html).not.toContain('<!--capsulebin:languages-->');
    });

    /**
     * The server turns each into a link or drops it (`api/assets.rs`); the label is
     * written here, in the shell's language, and escaped so it cannot end the comment.
     */
    it('carries one footer marker per legal page, labelled from its catalogue', () => {
        const catalogue = readCatalogue(lang);
        const footer = html.slice(html.indexOf('<footer'), html.indexOf('</footer>'));
        expect(footer.match(/<!--capsulebin:legal-link:/g)).toHaveLength(3);
        for (const page of ['imprint', 'privacy', 'terms']) {
            const label = catalogue[`shell.${page}`]!;
            expect(footer, page).toContain(`<!--capsulebin:legal-link:${page}:${label}-->`);
        }
        expect(html).not.toContain('<!--capsulebin:legal-links-->');
    });

    /** public/language-init.js and the language switch read it; the server adds nothing. */
    it('names every known language and the default in its head, before the head script', () => {
        const metas = doc.querySelectorAll<HTMLMetaElement>('meta[name="capsulebin-languages"]');
        expect(metas).toHaveLength(1);
        const meta = metas[0]!;
        expect(meta.parentElement).toBe(doc.head);
        expect(meta.content).toBe(KNOWN_LANGUAGES.join(' '));
        expect(meta.content.split(' ')[0]).toBe('en');
        expect(meta.dataset.default).toBe('en');

        const script = doc.head.querySelector('script[src="/language-init.js"]')!;
        expect(
            meta.compareDocumentPosition(script) & Node.DOCUMENT_POSITION_FOLLOWING,
        ).toBeTruthy();
    });

    it('links internally only within its language', () => {
        const links = [...doc.querySelectorAll<HTMLAnchorElement>('a[href^="/"]')].concat(
            [...doc.querySelectorAll('template')].flatMap((template) => [
                ...template.content.querySelectorAll<HTMLAnchorElement>('a[href^="/"]'),
            ]),
        );
        expect(links.length).toBeGreaterThan(0);
        for (const link of links) {
            expect(link.getAttribute('href')).toMatch(new RegExp(`^/${lang}/`));
        }
    });

    it('has a hidden language switch next to the theme toggle', () => {
        const tools = doc.querySelector('.cb-head-tools')!;
        expect(tools.querySelector('[data-language-switch][hidden]')).not.toBeNull();
        expect(tools.querySelector('[data-theme-toggle]')).not.toBeNull();
    });
});
