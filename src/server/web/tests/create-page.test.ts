/**
 * The create page's item tabs wired to the real HTML: adding items through the
 * menu, names, type changes that would lose content, removal, order, and what
 * ends up in the capsule.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { shellFor } from '../scripts/shells';

function el<T extends HTMLElement>(attr: string): T {
    return document.querySelector<T>(`[data-${attr}]`)!;
}

function tabs(): string[] {
    return [...document.querySelectorAll<HTMLElement>('[role="tab"]')].map(
        (tab) =>
            tab.querySelector('.cb-itemtab-name')!.textContent! +
            (tab.getAttribute('aria-selected') === 'true' ? ' *' : ''),
    );
}

function type(text: string): void {
    const area = el<HTMLTextAreaElement>('item-text');
    area.value = text;
    area.dispatchEvent(new Event('input'));
}

function add(kind: 'plain' | 'code' | 'markdown' | 'download'): void {
    el<HTMLButtonElement>('item-add').click();
    document.querySelector<HTMLButtonElement>(`[data-add-type="${kind}"]`)!.click();
}

async function loadPage(lang: 'en' | 'de' = 'en'): Promise<void> {
    document.documentElement.lang = lang;
    document.documentElement.innerHTML = new DOMParser().parseFromString(
        shellFor('index', lang),
        'text/html',
    ).documentElement.innerHTML;
    vi.resetModules();
    await import('../src/pages/create');
}

beforeEach(async () => {
    // jsdom has no layout and no ResizeObserver. Without widths, fitTabs keeps
    // every tab visible, which is what these tests assert against.
    vi.stubGlobal(
        'ResizeObserver',
        class {
            observe(): void {}
            disconnect(): void {}
        },
    );
    await loadPage();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('create page item tabs', () => {
    it('starts with one text item and no remove controls', () => {
        expect(tabs()).toEqual(['text.txt *']);
        expect(el('item-remove').hidden).toBe(true);
        expect(document.querySelector('[data-remove-id]')).toBeNull();
    });

    it('always shows the size line, so typing does not shift the layout', () => {
        const meter = el('size-meter');
        expect(meter.hidden).toBe(false);
        expect(meter.textContent).toMatch(/^0 B of /);
        type('x');
        expect(meter.textContent).toMatch(/^1 B of /);
    });

    it('replaces the untouched first item when another kind is added', () => {
        add('code');
        expect(tabs()).toEqual(['snippet.txt *']);
    });

    it('adds items in order, with unique default names', () => {
        type('one');
        add('markdown');
        type('# two');
        add('plain');
        type('three');
        add('plain');
        expect(tabs()).toEqual(['text.txt', 'notes.md', 'text-2.txt', 'text-3.txt *']);
        expect(el('size-meter').textContent).toContain('4 of 16 items');
    });

    it('renames code snippets with the language until the name is edited', () => {
        add('code');
        const lang = el<HTMLSelectElement>('item-lang');
        lang.value = 'rust';
        lang.dispatchEvent(new Event('change'));
        expect(tabs()).toEqual(['snippet.rs *']);

        const name = el<HTMLInputElement>('item-name');
        name.value = 'main.rs';
        name.dispatchEvent(new Event('input'));
        lang.value = 'python';
        lang.dispatchEvent(new Event('change'));
        expect(tabs()).toEqual(['main.rs *']);
    });

    it('strips path separators from names', () => {
        const name = el<HTMLInputElement>('item-name');
        name.value = '../etc/passwd';
        name.dispatchEvent(new Event('input'));
        expect(tabs()).toEqual(['.._etc_passwd *']);
    });

    it('keeps text when switching between text kinds, but never turns content into a file', () => {
        type('hello');
        const kind = el<HTMLSelectElement>('item-type');
        const file = [...kind.options].find((o) => o.value === 'download')!;
        expect(file.disabled).toBe(true);

        kind.value = 'markdown';
        kind.dispatchEvent(new Event('change'));
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('hello');
        expect(el('preview-switch').hidden).toBe(false);
    });

    it('removes an item with its tab button and selects a neighbour', () => {
        type('a');
        add('plain');
        type('b');
        add('plain');
        type('c');
        const second = document.querySelectorAll<HTMLButtonElement>('[data-remove-id]')[1]!;
        second.click();
        expect(tabs()).toEqual(['text.txt', 'text-3.txt *']);
    });

    it('moves between tabs with the arrow keys', () => {
        type('a');
        add('plain');
        type('b');
        const tab = () => document.activeElement as HTMLElement;
        const first = document.querySelector<HTMLElement>('[role="tab"]')!;
        first.focus();
        first.click();
        // The tabs are rebuilt on every change; focus must follow the selection.
        expect(tab().getAttribute('role')).toBe('tab');
        tab().dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
        expect(tabs()).toEqual(['text.txt', 'text-2.txt *']);
        expect(tab().textContent).toContain('text-2.txt');
        tab().dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }));
        expect(tabs()).toEqual(['text.txt *', 'text-2.txt']);
    });

    it('opens and closes the add menu', () => {
        const menu = el('add-menu');
        el<HTMLButtonElement>('item-add').click();
        expect(menu.hidden).toBe(false);
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
        expect(menu.hidden).toBe(true);
    });

    it('keeps the "more" menu hidden while every tab fits', () => {
        add('plain');
        add('plain');
        expect(el('more').hidden).toBe(true);
    });
});

describe('create page title', () => {
    it('offers an optional title field', () => {
        const title = document.querySelector<HTMLInputElement>('[data-capsule-title]')!;
        expect(title).not.toBeNull();
        expect(title.required).toBe(false);
        expect(title.maxLength).toBe(200);
    });
});

describe('create page in German', () => {
    beforeEach(async () => {
        await loadPage('de');
    });

    it('counts sizes and items in German', () => {
        type('x');
        expect(el('size-meter').textContent).toBe('1 B von 9,9 MiB');
        add('plain');
        expect(el('size-meter').textContent).toBe('2 von 16 Elementen · 1 B von 9,9 MiB');
    });

    it('labels the item controls in German', () => {
        add('code');
        expect(el<HTMLTextAreaElement>('item-text').placeholder).toBe('Code einfügen…');
        expect(document.querySelector('[role="tab"] .cb-itemtab-kind')!.textContent).toBe('Code');

        type('fn main() {}');
        add('download');
        const remove = document.querySelector<HTMLButtonElement>('[data-remove-id]')!;
        expect(remove.getAttribute('aria-label')).toBe('snippet.txt entfernen');
        expect(el<HTMLInputElement>('item-name').placeholder).toBe('Dateiname');
        expect(el('item-drop-label').textContent).toBe(
            'Datei hierher oder irgendwo auf dieses Formular ziehen',
        );
        expect(tabs()).toEqual(['snippet.txt', 'Datei *']);
    });
});
