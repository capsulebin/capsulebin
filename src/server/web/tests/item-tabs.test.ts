import { afterEach, describe, expect, it, vi } from 'vitest';
import { fitTabs, nextTabIndex } from '../src/pages/item-tabs';
import { downloadName } from '../src/pages/render-items';

describe('nextTabIndex', () => {
    it('moves with the arrow keys and wraps around', () => {
        expect(nextTabIndex('ArrowRight', 0, 3)).toBe(1);
        expect(nextTabIndex('ArrowRight', 2, 3)).toBe(0);
        expect(nextTabIndex('ArrowLeft', 0, 3)).toBe(2);
        expect(nextTabIndex('ArrowLeft', 2, 3)).toBe(1);
    });

    it('jumps to the ends with Home and End', () => {
        expect(nextTabIndex('Home', 2, 5)).toBe(0);
        expect(nextTabIndex('End', 0, 5)).toBe(4);
    });

    it('ignores other keys and empty lists', () => {
        expect(nextTabIndex('Enter', 1, 3)).toBeNull();
        expect(nextTabIndex('ArrowRight', 0, 0)).toBeNull();
    });
});

describe('downloadName', () => {
    it('uses the item name without any path', () => {
        expect(downloadName({ text: '', presentation: 'code', name: 'main.rs' }, 0)).toBe(
            'main.rs',
        );
        expect(downloadName({ text: '', presentation: 'plain', name: '../../etc/passwd' }, 0)).toBe(
            'passwd',
        );
        expect(downloadName({ text: '', presentation: 'plain', name: 'C:\\x\\a.txt' }, 0)).toBe(
            'a.txt',
        );
    });

    it('falls back to a generic name with a fitting extension', () => {
        expect(downloadName({ text: '', presentation: 'markdown' }, 1)).toBe('item-2.md');
        expect(downloadName({ text: '', presentation: 'plain', name: '  ' }, 0)).toBe('item-1.txt');
        expect(downloadName({ text: '', presentation: 'download' }, 2)).toBe('item-3.bin');
    });
});

describe('fitTabs', () => {
    afterEach(() => {
        vi.restoreAllMocks();
    });

    /** A bar of `count` tabs, each `tabWidth` wide, in a bar `width` wide; the button is 60. */
    function layout(count: number, width: number, tabWidth = 100) {
        const bar = document.createElement('div');
        for (let i = 0; i < count; i++) bar.append(document.createElement('button'));
        const more = document.createElement('button');
        vi.spyOn(bar, 'clientWidth', 'get').mockReturnValue(width);
        for (const tab of bar.children) {
            vi.spyOn(tab, 'getBoundingClientRect').mockReturnValue({ width: tabWidth } as DOMRect);
        }
        vi.spyOn(more, 'getBoundingClientRect').mockReturnValue({ width: 60 } as DOMRect);
        const visible = () => [...bar.children].map((tab) => !(tab as HTMLElement).hidden);
        return { bar, more, visible };
    }

    it('shows every tab and no button when they fit', () => {
        const { bar, more, visible } = layout(3, 300);
        expect(fitTabs(bar, more, 0)).toEqual([]);
        expect(more.hidden).toBe(true);
        expect(visible()).toEqual([true, true, true]);
    });

    it('shows the first tabs that fit and the button for the rest', () => {
        const { bar, more, visible } = layout(16, 300);
        expect(fitTabs(bar, more, 0)).toEqual([2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        expect(more.hidden).toBe(false);
        expect(visible().slice(0, 3)).toEqual([true, true, false]);
    });

    it('keeps the active tab visible in place of the last one', () => {
        const { bar, more, visible } = layout(16, 300);
        const hidden = fitTabs(bar, more, 11);
        expect(hidden).not.toContain(11);
        expect(hidden).toContain(1);
        expect(visible()[0]).toBe(true);
        expect(visible()[11]).toBe(true);
    });

    it('shows everything without a layout', () => {
        const { bar, more, visible } = layout(16, 0);
        expect(fitTabs(bar, more, 5)).toEqual([]);
        expect(more.hidden).toBe(true);
        expect(visible().every(Boolean)).toBe(true);
    });
});
