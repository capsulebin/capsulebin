/**
 * src/pages/render-items.ts: item kinds, names, sizes and the file card in the
 * page language.
 */

import { render } from 'lit';
import { describe, expect, it } from 'vitest';

import {
    itemMeta,
    itemName,
    kindBadge,
    renderItemContent,
    type DisplayItem,
} from '../src/pages/render-items';
import { setPageLanguage } from './page-language';

const file: DisplayItem = {
    text: '',
    presentation: 'download',
    name: 'report.pdf',
    bytes: new Uint8Array(1536),
};

describe('item display', () => {
    it('names kinds and sizes in English', async () => {
        await setPageLanguage('en');
        expect(kindBadge(file)).toBe('File');
        expect(itemName({ ...file, name: '' }, 1)).toBe('File 2');
        expect(itemMeta(file)).toBe('File · 1.5 KiB');
        expect(kindBadge({ text: '', presentation: 'future' })).toBe('Item');
    });

    it('names kinds and sizes in German', async () => {
        await setPageLanguage('de');
        expect(kindBadge(file)).toBe('Datei');
        expect(itemName({ ...file, name: '' }, 1)).toBe('Datei 2');
        expect(itemMeta(file)).toBe('Datei · 1,5 KiB');
        expect(itemMeta({ text: 'x', presentation: 'code', language: 'rust' })).toBe('rust · 0 B');
        expect(kindBadge({ text: '', presentation: 'future' })).toBe('Element');
    });

    it('offers a file for download in the page language', async () => {
        await setPageLanguage('de');
        const container = document.createElement('div');
        render(renderItemContent(file, { source: false, index: 0 }), container);
        expect(container.querySelector('button')!.textContent!.trim()).toBe(
            'report.pdf (1,5 KiB) herunterladen',
        );
    });

    it('shows content straight on the sheet, without a card', async () => {
        await setPageLanguage('en');
        for (const item of [
            file,
            { text: '# x', presentation: 'markdown' },
            { text: 'x', presentation: 'code', language: 'rust' },
            { text: 'x', presentation: 'plain' },
        ]) {
            expect(rendered(item).querySelector('.cb-card')).toBeNull();
        }
    });
});

/** Renders `item` (not as source) into a fresh container. */
function rendered(item: DisplayItem, source = false): HTMLElement {
    const container = document.createElement('div');
    render(renderItemContent(item, { source, index: 0 }), container);
    return container;
}

describe('line numbers', () => {
    const code = (text: string): DisplayItem => ({ text, presentation: 'code', language: 'rust' });

    it('numbers every line of a code item in a hidden column of its own', () => {
        const text = 'fn main() {\n    /* one\n       two */\n    let x = "a\nb";\n}\n';
        const container = rendered(code(text));

        const gutter = container.querySelector('.cb-code-gutter')!;
        expect(gutter.getAttribute('aria-hidden')).toBe('true');
        expect(gutter.textContent).toBe('1\n2\n3\n4\n5\n6');
        expect(gutter.contains(container.querySelector('code'))).toBe(false);

        // The code is exactly the item text, with multi-line tokens left whole.
        const codeElement = container.querySelector('code')!;
        expect(codeElement.textContent).toBe(text);
        const comments = codeElement.querySelectorAll('.hljs-comment');
        expect(comments).toHaveLength(1);
        expect(comments[0]!.textContent).toBe('/* one\n       two */');
    });

    it('counts lines as the code block shows them', () => {
        const numbers = (text: string) =>
            rendered(code(text)).querySelector('.cb-code-gutter')!.textContent!.split('\n');
        expect(numbers('a')).toEqual(['1']);
        expect(numbers('a\n')).toEqual(['1']);
        expect(numbers('a\nb')).toEqual(['1', '2']);
        expect(numbers('a\n\n')).toEqual(['1', '2']);
        expect(numbers('')).toEqual(['1']);
        expect(numbers('a\r\nb\r\n')).toEqual(['1', '2']);
    });

    it('leaves plain text and Markdown source without line numbers', () => {
        expect(
            rendered({ text: 'a\nb', presentation: 'plain' }).querySelector('.cb-code-gutter'),
        ).toBeNull();
        const source = rendered({ text: '# a\nb', presentation: 'markdown' }, true);
        expect(source.querySelector('.cb-code-gutter')).toBeNull();
        expect(source.querySelector('pre')!.textContent).toBe('# a\nb');
    });
});
