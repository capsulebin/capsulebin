import { describe, expect, it } from 'vitest';
import { nextTabIndex } from '../src/pages/item-tabs';
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
