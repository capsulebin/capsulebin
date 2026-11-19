/**
 * src/pages/render-items.ts: item kinds, names, sizes and the file card in the
 * page language.
 */

import { render } from 'lit';
import { afterEach, describe, expect, it } from 'vitest';

import {
    itemMeta,
    itemName,
    kindBadge,
    renderItemContent,
    type DisplayItem,
} from '../src/pages/render-items';

const file: DisplayItem = {
    text: '',
    presentation: 'download',
    name: 'report.pdf',
    bytes: new Uint8Array(1536),
};

afterEach(() => {
    document.documentElement.lang = '';
});

describe('item display', () => {
    it('names kinds and sizes in English', () => {
        document.documentElement.lang = 'en';
        expect(kindBadge(file)).toBe('File');
        expect(itemName({ ...file, name: '' }, 1)).toBe('File 2');
        expect(itemMeta(file)).toBe('File · 1.5 KiB');
        expect(kindBadge({ text: '', presentation: 'future' })).toBe('Item');
    });

    it('names kinds and sizes in German', () => {
        document.documentElement.lang = 'de';
        expect(kindBadge(file)).toBe('Datei');
        expect(itemName({ ...file, name: '' }, 1)).toBe('Datei 2');
        expect(itemMeta(file)).toBe('Datei · 1,5 KiB');
        expect(itemMeta({ text: 'x', presentation: 'code', language: 'rust' })).toBe('rust · 0 B');
        expect(kindBadge({ text: '', presentation: 'future' })).toBe('Element');
    });

    it('offers a file for download in the page language', () => {
        document.documentElement.lang = 'de';
        const container = document.createElement('div');
        render(renderItemContent(file, { source: false, index: 0 }), container);
        expect(container.querySelector('button')!.textContent!.trim()).toBe(
            'report.pdf (1,5 KiB) herunterladen',
        );
    });
});
