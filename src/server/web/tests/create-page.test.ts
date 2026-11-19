/**
 * The create page's item tabs wired to the real HTML: adding items through the
 * menu, names, type changes that would lose content, removal, order, and what
 * ends up in the capsule.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { servedShell, setPageLanguage } from './page-language';

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
    area.dispatchEvent(new Event('input', { bubbles: true }));
}

function add(kind: 'plain' | 'code' | 'markdown' | 'download'): void {
    el<HTMLButtonElement>('item-add').click();
    document.querySelector<HTMLButtonElement>(`[data-add-type="${kind}"]`)!.click();
}

async function loadPage(lang: 'en' | 'de' = 'en'): Promise<void> {
    document.documentElement.innerHTML = new DOMParser().parseFromString(
        servedShell('index', lang),
        'text/html',
    ).documentElement.innerHTML;
    vi.resetModules();
    await setPageLanguage(lang);
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

describe('create page language switch', () => {
    // AC27: the notice is in the chosen language, on an English page too.
    const GERMAN_NOTICE = 'Die Sprache gilt ab der nächsten Seite.';

    function chooseGerman(): void {
        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="de"]')!.click();
    }

    function notice(): string | null | undefined {
        return document.querySelector('[data-language-notice]')?.textContent;
    }

    afterEach(() => {
        localStorage.clear();
    });

    // jsdom cannot navigate and location.replace cannot be spied on: the notice
    // shows the switch's mode here, e2e/i18n.e2e.ts covers the navigation itself.
    it('navigates on an empty form', () => {
        chooseGerman();
        expect(notice()).toBeUndefined();
    });

    it('only stores the choice once an item holds text', () => {
        type('draft');
        chooseGerman();
        expect(localStorage.getItem('capsulebin.language')).toBe('de');
        expect(notice()).toBe(GERMAN_NOTICE);
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('draft');
    });

    it('only stores the choice once an item has a name', () => {
        const name = el<HTMLInputElement>('item-name');
        name.value = 'todo.txt';
        name.dispatchEvent(new Event('input', { bubbles: true }));
        chooseGerman();
        expect(notice()).toBe(GERMAN_NOTICE);
    });

    // AC24 acceptance: "any item holds content (text, name or a file)".
    it('only stores the choice once a file is chosen', async () => {
        const input = el<HTMLInputElement>('item-file');
        const file = new File([new Uint8Array([1, 2, 3])], 'data.bin');
        Object.defineProperty(input, 'files', { configurable: true, value: [file] });
        input.dispatchEvent(new Event('change'));
        await vi.waitFor(() => expect(tabs()).toEqual(['data.bin *']));
        chooseGerman();
        expect(localStorage.getItem('capsulebin.language')).toBe('de');
        expect(notice()).toBe(GERMAN_NOTICE);
        expect(tabs()).toEqual(['data.bin *']);
    });

    it('only stores the choice once a file is dropped on the form', async () => {
        const form = el<HTMLFormElement>('create-form');
        const drop = new Event('drop', { bubbles: true, cancelable: true });
        Object.defineProperty(drop, 'dataTransfer', {
            value: { files: [new File(['x'], 'dropped.txt')], types: ['Files'] },
        });
        form.dispatchEvent(drop);
        await vi.waitFor(() => expect(tabs()).toEqual(['dropped.txt *']));
        chooseGerman();
        expect(notice()).toBe(GERMAN_NOTICE);
    });

    it('still navigates after only adding further empty items', () => {
        add('code');
        add('markdown');
        chooseGerman();
        expect(notice()).toBeUndefined();
    });

    // AC24: any other form field changed from its default is a draft too.
    function change(
        id: string,
        set: (control: HTMLInputElement & HTMLSelectElement) => void,
    ): void {
        const control = document.getElementById(id) as HTMLInputElement & HTMLSelectElement;
        set(control);
        control.dispatchEvent(new Event('input', { bubbles: true }));
        control.dispatchEvent(new Event('change', { bubbles: true }));
    }

    it.each([
        ['the title', 'create-title', 'Meeting notes'],
        ['the password', 'create-password', 'secret'],
        ['the expiry', 'create-expiry', '3600'],
        ['no expiry', 'create-expiry', ''],
    ])('only stores the choice once %s is set', (_field, id, value) => {
        change(id, (control) => (control.value = value));
        chooseGerman();
        expect(localStorage.getItem('capsulebin.language')).toBe('de');
        expect(notice()).toBe(GERMAN_NOTICE);
        expect((document.getElementById(id) as HTMLInputElement).value).toBe(value);
    });

    it('only stores the choice once burn after reading is ticked', () => {
        change('create-burn', (control) => (control.checked = true));
        chooseGerman();
        expect(notice()).toBe(GERMAN_NOTICE);
        expect((document.getElementById('create-burn') as HTMLInputElement).checked).toBe(true);
    });

    it('still navigates when a field is back at its default', () => {
        change('create-expiry', (control) => (control.value = '604800'));
        change('create-burn', (control) => (control.checked = false));
        change('create-title', (control) => (control.value = ''));
        chooseGerman();
        expect(notice()).toBeUndefined();
    });

    it('still navigates after only changing the type of an empty item', () => {
        change('create-item-type', (control) => (control.value = 'code'));
        change('create-item-lang', (control) => (control.value = 'rust'));
        chooseGerman();
        expect(notice()).toBeUndefined();
    });

    it('only stores the choice in a German page too, with the notice in the chosen language', async () => {
        await loadPage('de');
        type('Entwurf');
        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!.click();
        expect(localStorage.getItem('capsulebin.language')).toBe('en');
        expect(notice()).toBe('The language applies from the next page on.');
        expect(document.querySelector('[data-language-notice]')!.getAttribute('lang')).toBe('en');
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('Entwurf');
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
