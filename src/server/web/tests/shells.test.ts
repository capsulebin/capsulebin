/**
 * The shell templates and the per-language shells generated from them
 * (scripts/shells.ts): every visible text comes from the catalogue, nothing of
 * the template machinery is left over, the server's markers and tokens are all
 * there, the languages meta is written at build time, and every internal link
 * stays within the page's language once the server has filled in the prefix.
 */

import { KNOWN_LANGUAGES } from '@capsulebin/core';
import { describe, expect, it } from 'vitest';

import {
    CREATION_TOKEN,
    DEFAULT_BRANDING,
    DEFAULT_TOKEN,
    FAVICON_TOKEN,
    NAME_PLACEHOLDER,
    NAME_TOKEN,
    PREFIX_TOKEN,
    PROJECT_LINK,
    PROJECT_PLACEHOLDER,
    SOURCE_TOKEN,
    SHELLS,
    catalogueLanguages,
    fillServerTokens,
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
 * Literals that may stay in a template: symbols, and the names of the
 * highlight.js languages in the code-language menu. Not the product name: the
 * instance's name is the server's to fill in.
 */
const ALLOWED_LITERALS = new Set([
    '·',
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

/** The server's markers and how often each shell carries them (title: `<title>`, header, `<h1>`). */
const MARKERS = {
    '<!--capsulebin:legal-title-->': (shell: string) => (shell === 'legal' ? 3 : 0),
    '<!--capsulebin:legal-content-->': (shell: string) => (shell === 'legal' ? 1 : 0),
    '<!--capsulebin:logo-->': () => 1,
    '<!--/capsulebin:logo-->': () => 1,
    '<!--capsulebin:wordmark-->': () => 1,
    '<!--/capsulebin:wordmark-->': () => 1,
    '<!--capsulebin:tagline-->': (shell: string) => (shell === 'index' ? 1 : 0),
    '<!--/capsulebin:tagline-->': (shell: string) => (shell === 'index' ? 1 : 0),
};

/** The catalogue texts that name the service, through {@link NAME_PLACEHOLDER}, and their shell. */
const NAMING_TEXTS: Readonly<Record<string, string>> = {
    'create.noscript': 'index',
    'view.noscript': 'p',
    'report.back': 'report',
};

/** How often a generated shell carries the name token: `<title>`, header, naming text. */
function nameTokens(shell: string): number {
    return 2 + Object.values(NAMING_TEXTS).filter((owner) => owner === shell).length;
}

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

    it.each(SHELLS)('%s.html does not name the product', (shell) => {
        expect(readTemplate(shell)).not.toContain('CapsuleBin');
    });

    it.each(SHELLS)('%s.html loads the head scripts in order, blocking', (shell) => {
        const html = readTemplate(shell);
        const head = html.slice(0, html.indexOf('</head>'));
        const order = [
            'name="capsulebin-languages"',
            '<meta name="color-scheme" content="light dark" />',
            '<link rel="icon" href="{{favicon}}" />',
            '<script src="/theme-init.js"></script>',
            '<script src="/language-init.js"></script>',
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

    it('turns the name placeholder into the name token after escaping', () => {
        expect(renderShell('<p>{{a.b}}</p>', 'en', { 'a.b': '<{name}> & {name}' })).toBe(
            `<p>&lt;${NAME_TOKEN}&gt; &amp; ${NAME_TOKEN}</p>`,
        );
    });

    it('turns the project placeholder into the project link after escaping', () => {
        expect(renderShell('<p>{{a.b}}</p>', 'en', { 'a.b': '<b> & {project}' })).toBe(
            `<p>&lt;b&gt; &amp; ${PROJECT_LINK}</p>`,
        );
        expect(PROJECT_LINK).toBe(
            `<a href="${SOURCE_TOKEN}" target="_blank" rel="noopener noreferrer">CapsuleBin</a>`,
        );
    });

    it('fails on the project placeholder inside a comment', () => {
        expect(() =>
            renderShell('<!--capsulebin:legal-link:terms:{{a.b}}-->', 'de', { 'a.b': '{project}' }),
        ).toThrow(/de\.json.*\{\{a\.b\}\}.*comment.*\{project\}/);
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

    /**
     * Every `{…}` in a text placed into a shell becomes the name token, the
     * project link or stays literally, so only the texts that name the service
     * may have one, and only the name, and only the attribution names the project.
     */
    it('names the service through the name placeholder in exactly the naming texts', () => {
        const placed = new Set(
            SHELLS.flatMap((shell) =>
                [...readTemplate(shell).matchAll(/\{\{([A-Za-z0-9_]+\.[A-Za-z0-9_.]+)\}\}/g)].map(
                    (match) => match[1]!,
                ),
            ),
        );
        const withParameter = [...placed].filter((key) => /\{\w+\}/.test(catalogue[key] ?? ''));
        expect(withParameter.sort()).toEqual(
            [...Object.keys(NAMING_TEXTS), 'shell.poweredBy'].sort(),
        );
        for (const key of Object.keys(NAMING_TEXTS)) {
            expect(catalogue[key]!.match(/\{\w+\}/g), key).toEqual([NAME_PLACEHOLDER]);
            expect(catalogue[key], key).not.toContain('CapsuleBin');
        }
    });

    /** The attribution names the project, whatever the instance is called, through the link. */
    it('names the project in the attribution through the project placeholder', () => {
        expect(catalogue['shell.poweredBy']!.match(/\{\w+\}/g)).toEqual([PROJECT_PLACEHOLDER]);
        expect(catalogue['shell.poweredBy']).not.toContain('CapsuleBin');
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

    /** The server replaces the region with a configured logo (`api/assets.rs`). */
    it('wraps the built-in symbol, and only it, in the logo markers', () => {
        const region = html.slice(
            html.indexOf('<!--capsulebin:logo-->') + '<!--capsulebin:logo-->'.length,
            html.indexOf('<!--/capsulebin:logo-->'),
        );
        const symbol = parse(region).body;
        expect(symbol.children).toHaveLength(1);
        expect(symbol.firstElementChild!.tagName).toBe('svg');
        expect(symbol.textContent!.trim()).toBe('');
        expect(doc.querySelectorAll('.cb-logo svg')).toHaveLength(1);
    });

    /** The server drops the region when the name is hidden behind a logo. */
    it('wraps the name in the header in the wordmark markers', () => {
        const region = html.slice(
            html.indexOf('<!--capsulebin:wordmark-->') + '<!--capsulebin:wordmark-->'.length,
            html.indexOf('<!--/capsulebin:wordmark-->'),
        );
        expect(region.trim()).toBe(`<span class="cb-logo-name">${NAME_TOKEN}</span>`);
        expect(html.indexOf('<!--/capsulebin:logo-->')).toBeLessThan(
            html.indexOf('<!--capsulebin:wordmark-->'),
        );
        expect(doc.querySelector('.cb-logo')!.textContent!.trim()).toBe(NAME_TOKEN);
    });

    it('takes its title and the texts naming the service from the name token', () => {
        expect(doc.title.endsWith(NAME_TOKEN)).toBe(true);
        expect(html.split(NAME_TOKEN)).toHaveLength(nameTokens(shell) + 1);
        expect(html).not.toContain(NAME_PLACEHOLDER);
    });

    it('links the favicon from the favicon token', () => {
        const icons = doc.head.querySelectorAll('link[rel="icon"]');
        expect(icons).toHaveLength(1);
        expect(icons[0]!.getAttribute('href')).toBe(FAVICON_TOKEN);
        expect(html.split(FAVICON_TOKEN)).toHaveLength(2);
    });

    /**
     * AGPL section 13: every page offers the source code, as the last footer item.
     * Only the project's name is the link, opened in a new tab; the rest of the
     * text stays outside it.
     */
    it('ends the footer links with the attribution, only the project name linked', () => {
        const attribution = doc.querySelector('.cb-foot-links')!.lastElementChild!;
        expect(attribution.tagName).toBe('SPAN');
        expect(attribution.textContent).toBe(
            readCatalogue(lang)['shell.poweredBy']!.replace(PROJECT_PLACEHOLDER, 'CapsuleBin'),
        );
        const links = attribution.querySelectorAll('a');
        expect(links).toHaveLength(1);
        const link = links[0]!;
        expect(link.getAttribute('href')).toBe(SOURCE_TOKEN);
        expect(link.getAttribute('target')).toBe('_blank');
        expect(link.getAttribute('rel')).toBe('noopener noreferrer');
        expect(link.textContent).toBe('CapsuleBin');
        const [before, after] = readCatalogue(lang)['shell.poweredBy']!.split(PROJECT_PLACEHOLDER);
        expect(link.previousSibling?.textContent ?? '').toBe(before);
        expect(link.nextSibling?.textContent ?? '').toBe(after);
        expect(html.split(SOURCE_TOKEN)).toHaveLength(2);
    });

    /** The server replaces the region with a configured tagline or drops it (`api/assets.rs`). */
    it('wraps the slogan of the create page, and only it, in the tagline markers', () => {
        if (shell !== 'index') {
            expect(html).not.toContain('capsulebin:tagline');
            return;
        }
        const region = html.slice(
            html.indexOf('<!--capsulebin:tagline-->') + '<!--capsulebin:tagline-->'.length,
            html.indexOf('<!--/capsulebin:tagline-->'),
        );
        expect(region.trim()).toBe(
            `<span class="cb-claim">${readCatalogue(lang)['create.claim']}</span>`,
        );
        expect(doc.querySelectorAll('.cb-claim')).toHaveLength(1);
        expect(html.indexOf('<!--/capsulebin:wordmark-->')).toBeLessThan(
            html.indexOf('<!--capsulebin:tagline-->'),
        );
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

    /**
     * public/language-init.js and the language switch read it. The languages are
     * written here; the default language is the server's to fill in.
     */
    it('names every known language in its head, before the head script, with the default token', () => {
        const metas = doc.querySelectorAll<HTMLMetaElement>('meta[name="capsulebin-languages"]');
        expect(metas).toHaveLength(1);
        const meta = metas[0]!;
        expect(meta.parentElement).toBe(doc.head);
        expect(meta.content).toBe(KNOWN_LANGUAGES.join(' '));
        expect(meta.dataset.default).toBe(DEFAULT_TOKEN);
        expect(html.split(DEFAULT_TOKEN)).toHaveLength(2);

        const script = doc.head.querySelector('script[src="/language-init.js"]')!;
        expect(
            meta.compareDocumentPosition(script) & Node.DOCUMENT_POSITION_FOLLOWING,
        ).toBeTruthy();
    });

    /**
     * The create page reads the instance's creation mode from it before anything
     * is written; the mode is configuration, so the server fills it in. No other
     * page carries it.
     */
    it('carries the creation meta with the creation token on the create page only', () => {
        const metas = doc.querySelectorAll<HTMLMetaElement>('meta[name="capsulebin-creation"]');
        if (shell !== 'index') {
            expect(metas).toHaveLength(0);
            expect(html).not.toContain('capsulebin-creation');
            expect(html).not.toContain(CREATION_TOKEN);
            return;
        }
        expect(metas).toHaveLength(1);
        expect(metas[0]!.parentElement).toBe(doc.head);
        expect(metas[0]!.content).toBe(CREATION_TOKEN);
        expect(html.split(CREATION_TOKEN)).toHaveLength(2);
    });

    /** Every `<a href>` of `page`, including those inside <template>s. */
    const allHrefs = (page: Document): string[] =>
        [...page.querySelectorAll('a[href]')]
            .concat(
                [...page.querySelectorAll('template')].flatMap((template) => [
                    ...template.content.querySelectorAll('a[href]'),
                ]),
            )
            .map((link) => link.getAttribute('href')!);

    /** The internal links: all but the attribution, which points to the source code. */
    const hrefs = (page: Document): string[] =>
        allHrefs(page).filter((href) => href !== SOURCE_TOKEN && href !== DEFAULT_BRANDING.source);

    /** The prefix depends on the default language, so the server fills it in. */
    it('starts every link with the prefix token and has no language in a link', () => {
        const links = hrefs(doc);
        expect(links.length).toBeGreaterThan(0);
        for (const href of links) {
            expect(href.startsWith(`${PREFIX_TOKEN}/`), href).toBe(true);
            expect(href.slice(PREFIX_TOKEN.length), href).not.toMatch(
                new RegExp(`^/(${KNOWN_LANGUAGES.join('|')})(/|$)`),
            );
        }
        expect(html).not.toContain(`/${lang}/`);
    });

    it('starts every link but the attribution with the prefix token', () => {
        const outside = allHrefs(doc).filter((href) => !href.startsWith(`${PREFIX_TOKEN}/`));
        expect(outside).toEqual([SOURCE_TOKEN]);
    });

    it('carries the tokens only where the server fills them', () => {
        const other = html
            .replaceAll(`href="${PREFIX_TOKEN}/`, '')
            .replaceAll(`data-default="${DEFAULT_TOKEN}"`, '')
            .replaceAll(NAME_TOKEN, '')
            .replaceAll(`<link rel="icon" href="${FAVICON_TOKEN}" />`, '')
            .replaceAll(`<meta name="capsulebin-creation" content="${CREATION_TOKEN}" />`, '')
            .replaceAll(PROJECT_LINK, '');
        expect(other).not.toContain('%%');
    });

    it.each(KNOWN_LANGUAGES)(
        'links only within its language on an instance with default %s',
        (d) => {
            const served = fillServerTokens(html, lang, d);
            expect(served).not.toContain('%%');
            expect(served).toContain(`<link rel="icon" href="${DEFAULT_BRANDING.favicon}" />`);
            expect(served).toContain(PROJECT_LINK.replace(SOURCE_TOKEN, DEFAULT_BRANDING.source));
            expect(parse(served).title).toMatch(new RegExp(`${DEFAULT_BRANDING.name}$`));
            expect(
                parse(served).querySelector<HTMLMetaElement>('meta[name="capsulebin-languages"]')!
                    .dataset.default,
            ).toBe(d);
            expect(
                parse(served).querySelector<HTMLMetaElement>('meta[name="capsulebin-creation"]')
                    ?.content,
            ).toBe(shell === 'index' ? 'public' : undefined);

            const others = KNOWN_LANGUAGES.filter((known) => known !== lang).join('|');
            for (const href of hrefs(parse(served))) {
                if (lang === d) {
                    expect(href, href).toMatch(/^\/(?!\/)/);
                    expect(href, href).not.toMatch(
                        new RegExp(`^/(${KNOWN_LANGUAGES.join('|')})(/|$)`),
                    );
                } else {
                    expect(href, href).toMatch(new RegExp(`^/${lang}/`));
                }
                expect(href, href).not.toMatch(new RegExp(`^/(${others})(/|$)`));
            }
        },
    );

    it('has a hidden language switch next to the theme toggle', () => {
        const tools = doc.querySelector('.cb-head-tools')!;
        expect(tools.querySelector('[data-language-switch][hidden]')).not.toBeNull();
        expect(tools.querySelector('[data-theme-toggle]')).not.toBeNull();
    });
});
