/**
 * The English view page on real capsules from the in-memory API: every state
 * phase with its hexagon, and the display as one sheet - chip row, actions,
 * copied state, "All N" list and previous/next row. The page defines its custom
 * element once per document, so the file loads the page once and mounts a fresh
 * `<cb-view-page>` per capsule.
 */

import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { Paste, PasteApi, generateUlid } from '@capsulebin/core';

import { createFakeServer } from '../../../core/tests/fake-server';
import en from '../src/i18n/en.json';
import { formatSize } from '../src/language';
import { COPIED_FOR_MS } from '../src/pages/copy-state';
import { servedShell, setPageLanguage } from './page-language';

const server = createFakeServer();
/** A well-formed link for an id the server does not know. */
const UNKNOWN = `/p/AbCdEfGhIjKlMnOpQrStUv#v1.${'A'.repeat(43)}`;
/** What `fetch` does for the next requests; the fake server unless a test says otherwise. */
let respond: typeof fetch = server.fetch;

interface Item {
    name: string;
    text: string;
    presentation?: string;
    language?: string;
}

interface Options {
    burn?: boolean;
    title?: string | null;
    password?: string | null;
}

/** Creates a capsule on the fake server and returns its share path with the key. */
async function publish(items: readonly Item[], options: Options = {}): Promise<string> {
    const paste = new Paste({
        title: options.title ?? null,
        password: options.password ?? null,
        expiresIn: null,
        burnAfterRetrieval: options.burn ?? false,
    });
    for (const item of items) {
        const bytes = new TextEncoder().encode(item.text);
        paste.addItem({
            id: generateUlid(),
            name: item.name,
            size: bytes.length,
            mediaType: 'text/plain',
            presentation: item.presentation ?? 'plain',
            ...(item.language ? { language: item.language } : {}),
            bytes,
        });
    }
    const { shareUrl } = await paste.publish(new PasteApi(''), '');
    return shareUrl;
}

async function flush(): Promise<void> {
    for (let i = 0; i < 20; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

/** Replaces the page's component with a fresh one opening `path`. */
function mount(path: string): HTMLElement {
    document.querySelector('cb-view-page')?.remove();
    window.history.replaceState(null, '', path);
    const page = document.createElement('cb-view-page');
    document.querySelector('[data-view-root]')!.append(page);
    return page;
}

/** Opens a capsule up to the display (confirming a burn warning on the way). */
async function show(items: readonly Item[], options: Options = {}): Promise<HTMLElement> {
    const page = mount(await publish(items, options));
    if (options.burn) {
        const confirm = await vi.waitFor(() => {
            const button = [...page.querySelectorAll('button')].find(
                (candidate) => candidate.textContent?.trim() === en['view.burnConfirm'],
            );
            if (!button) throw new Error('no burn warning yet');
            return button;
        });
        confirm.click();
    }
    await vi.waitFor(() => expect(page.querySelector('[data-sheet]')).not.toBeNull(), {
        timeout: 5000,
    });
    return page;
}

function texts(elements: Iterable<Element>): string[] {
    return [...elements].map((element) => element.textContent!.replace(/\s+/g, ' ').trim());
}

const THREE: readonly Item[] = [
    { name: 'notes.md', text: '# Notes\n\nSome *text*.\n', presentation: 'markdown' },
    { name: 'main.rs', text: 'fn main() {}\n', presentation: 'code', language: 'rust' },
    { name: 'todo.txt', text: 'one\ntwo\n' },
];

/**
 * Pretends a layout in which the tab list is `width` pixels wide, every tab 100
 * and the "All N" button 60, and the panel starts above the sheet's sticky bar
 * (the page is scrolled past it). Returns the spy on `window.scrollBy`.
 */
function stubLayout(width: number) {
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
        this: HTMLElement,
    ) {
        return this.getAttribute('role') === 'tablist' ? width : 0;
    });
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (
        this: Element,
    ) {
        const rect = { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0 };
        if (this.getAttribute('role') === 'tab') rect.width = 100;
        if (this.hasAttribute('data-items-all')) rect.width = 60;
        if (this.classList.contains('cb-sheet-bar')) rect.bottom = 40;
        if (this.classList.contains('cb-item-panel')) rect.top = -500;
        return rect as DOMRect;
    });
    return vi.spyOn(window, 'scrollBy').mockImplementation(() => {});
}

beforeAll(async () => {
    localStorage.clear();
    // jsdom has neither layout nor ResizeObserver.
    vi.stubGlobal(
        'ResizeObserver',
        class {
            observe(): void {}
            disconnect(): void {}
        },
    );
    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => respond(input, init));
    respond = () => new Promise<Response>(() => {});

    const page = new DOMParser().parseFromString(servedShell('p', 'en'), 'text/html');
    await setPageLanguage('en');
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', UNKNOWN);
    await import('../src/pages/view');
    await flush();
});

afterEach(() => {
    respond = server.fetch;
    vi.useRealTimers();
    vi.restoreAllMocks();
});

afterAll(() => {
    vi.unstubAllGlobals();
});

describe('view page states', () => {
    function hexagons(page: HTMLElement): string[] {
        return [...page.querySelectorAll<SVGElement>('[data-hexagon]')].map(
            (hexagon) => hexagon.dataset.hexagon!,
        );
    }

    it('runs the hexagon while opening', async () => {
        respond = () => new Promise<Response>(() => {});
        const page = mount(UNKNOWN);
        await flush();
        expect(page.querySelector('h1')!.textContent).toBe(en['view.opening']);
        expect(hexagons(page)).toEqual(['running']);
    });

    it('keeps the running hexagon node across a re-render', async () => {
        respond = () => new Promise<Response>(() => {});
        const page = mount(UNKNOWN) as HTMLElement & { requestUpdate(): void };
        await flush();
        const running = page.querySelector('[data-hexagon="running"]');
        expect(running).not.toBeNull();

        page.requestUpdate();
        await flush();
        // A new node would restart the dash animation.
        expect(page.querySelector('[data-hexagon="running"]')).toBe(running);
    });

    it('runs the hexagon while decrypting', async () => {
        const path = await publish([{ name: 'a.txt', text: 'a' }]);
        respond = (input, init) =>
            String(input).endsWith('/retrieve')
                ? new Promise<Response>(() => {})
                : server.fetch(input, init);
        const page = mount(path);
        await vi.waitFor(() =>
            expect(page.querySelector('h1')!.textContent).toBe(en['view.decrypting']),
        );
        expect(hexagons(page)).toEqual(['running']);
    });

    it('locks the hexagon for a password and runs one while the key is derived', async () => {
        const page = mount(await publish([{ name: 'a.txt', text: 'secret' }], { password: 'pw' }));
        await vi.waitFor(() => expect(page.querySelector('#pwd-input')).not.toBeNull());
        expect(hexagons(page)).toEqual(['locked']);
        expect(page.querySelector('.cb-kdf-progress')!.hasAttribute('hidden')).toBe(true);

        page.querySelector<HTMLInputElement>('#pwd-input')!.value = 'pw';
        page.querySelector('form')!.requestSubmit();
        await vi.waitFor(() =>
            expect(page.querySelector('.cb-kdf-progress [data-hexagon="running"]')).not.toBeNull(),
        );
        expect(page.querySelector('.cb-kdf-progress')!.hasAttribute('hidden')).toBe(false);
        expect(hexagons(page)).toEqual(['locked', 'running']);

        await vi.waitFor(() => expect(page.querySelector('[data-sheet]')).not.toBeNull(), {
            timeout: 10_000,
        });
        expect(hexagons(page)).toEqual([]);
    });

    it('warns with the hexagon before a burn retrieval', async () => {
        const page = mount(await publish([{ name: 'a.txt', text: 'a' }], { burn: true }));
        await vi.waitFor(() =>
            expect(page.querySelector('h1')!.textContent).toBe(en['view.burnWarning']),
        );
        expect(hexagons(page)).toEqual(['warning']);
    });

    it('shows an empty hexagon for an already retrieved capsule', async () => {
        const path = await publish([{ name: 'a.txt', text: 'a' }]);
        respond = (input, init) =>
            String(input).endsWith('/retrieve')
                ? Promise.resolve(
                      new Response(JSON.stringify({ status: 410, code: 'gone' }), { status: 410 }),
                  )
                : server.fetch(input, init);
        const page = mount(path);
        await vi.waitFor(() =>
            expect(page.querySelector('h1')!.textContent).toBe(en['view.burned']),
        );
        expect(hexagons(page)).toEqual(['empty']);
    });

    it('shows an empty hexagon for an unavailable capsule', async () => {
        const page = mount(UNKNOWN);
        await vi.waitFor(() =>
            expect(page.querySelector('h1')!.textContent).toBe(en['view.unavailable']),
        );
        expect(hexagons(page)).toEqual(['empty']);
        expect(document.querySelector('.cb-spinner')).toBeNull();
    });
});

describe('view page display', () => {
    it.each([
        { burn: false, count: 1 },
        { burn: true, count: 1 },
        { burn: false, count: 3 },
        { burn: true, count: 3 },
    ])(
        'puts the chips between title and sheet (burn $burn, $count items)',
        async ({ burn, count }) => {
            const items = THREE.slice(0, count);
            const page = await show(items, { burn, title: 'Deploy notes' });

            const h1 = page.querySelector('h1')!;
            const sheet = page.querySelector('[data-sheet]')!;
            const rows = page.querySelectorAll('.cb-chips');
            expect(rows).toHaveLength(1);
            const row = rows[0]!;
            expect(h1.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
            expect(
                row.compareDocumentPosition(sheet) & Node.DOCUMENT_POSITION_FOLLOWING,
            ).toBeTruthy();

            const total = items.reduce(
                (sum, item) => sum + new TextEncoder().encode(item.text).length,
                0,
            );
            const size = count > 1 ? `${count} items · ${formatSize(total)}` : formatSize(total);
            expect(texts(row.children)).toEqual([
                ...(burn ? [en['view.deletedAfterRetrieval']] : []),
                en['expiry.none'],
                size,
                en['view.decryptedLocally'],
            ]);
            expect(row.children[0]!.classList.contains('cb-chip-warning')).toBe(burn);
            expect(row.querySelectorAll('.cb-chip-warning')).toHaveLength(burn ? 1 : 0);
            expect(page.querySelector('.cb-capsule-status')).toBeNull();
        },
    );

    it('holds tabs, actions, panel and the previous/next row in one sheet', async () => {
        const page = await show(THREE);
        const sheet = page.querySelector('[data-sheet]')!;
        expect(page.querySelectorAll('[data-sheet]')).toHaveLength(1);
        expect(sheet.querySelector('[role="tablist"]')).not.toBeNull();
        expect(sheet.querySelector('.cb-bar-actions')).not.toBeNull();
        expect(sheet.querySelector('.cb-item-panel')).not.toBeNull();
        expect(sheet.querySelector('.cb-item-nav')).not.toBeNull();
        expect(page.querySelector('.cb-card')).toBeNull();

        // Icon and label on every action; the label is what narrow screens hide.
        const actions = [...sheet.querySelectorAll<HTMLButtonElement>('.cb-bar-actions .cb-quiet')];
        expect(texts(actions)).toEqual([en['view.copy'], en['view.download']]);
        for (const action of actions) {
            expect(action.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
            expect(action.querySelector('.cb-bar-label')!.textContent).toBe(
                action.textContent!.trim(),
            );
        }
    });

    it('keeps chip and action icons across a tab switch', async () => {
        const page = await show(THREE);
        const icons = () => [...page.querySelectorAll('.cb-chips svg, .cb-bar-actions svg')];
        const before = icons();
        expect(before).toHaveLength(5);

        page.querySelector<HTMLButtonElement>('#cb-item-tab-2')!.click();
        await flush();
        const after = icons();
        expect(after).toHaveLength(before.length);
        after.forEach((node, i) => expect(node).toBe(before[i]));
    });

    it('switches Markdown between preview and source', async () => {
        const page = await show(THREE);
        const [preview, source] =
            page.querySelectorAll<HTMLButtonElement>('.cb-view-switch button');
        expect(preview!.getAttribute('aria-pressed')).toBe('true');
        expect(page.querySelector('.cb-item-panel .cb-prose h1')!.textContent).toBe('Notes');

        source!.click();
        await flush();
        expect(source!.getAttribute('aria-pressed')).toBe('true');
        expect(page.querySelector('.cb-item-panel .cb-pre')!.textContent).toBe(THREE[0]!.text);
        // Markdown source is text, not code: no line numbers.
        expect(page.querySelector('.cb-code-gutter')).toBeNull();
    });

    it('shows one item without tabs, list or previous/next row', async () => {
        const page = await show([THREE[1]!]);
        const sheet = page.querySelector('[data-sheet]')!;
        expect(sheet.querySelector('[role="tablist"]')).toBeNull();
        expect(sheet.querySelector('[data-items-all]')).toBeNull();
        expect(sheet.querySelector('.cb-item-nav')).toBeNull();
        expect(sheet.querySelector('#cb-item-name')!.textContent).toBe('main.rs');
        expect(sheet.querySelector('.cb-code-gutter')!.textContent).toBe('1');
    });

    it('copies an item and says so for two seconds', async () => {
        const writeText = vi.fn(async (_text: string) => {});
        Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
        const page = await show(THREE);
        page.querySelector<HTMLButtonElement>('#cb-item-tab-1')!.click();
        await flush();

        vi.useFakeTimers();
        const button = page.querySelector<HTMLButtonElement>('[data-item-copy]')!;
        button.click();
        await vi.advanceTimersByTimeAsync(0);

        expect(writeText).toHaveBeenCalledExactlyOnceWith(THREE[1]!.text);
        expect(button.classList.contains('cb-copied')).toBe(true);
        expect(button.querySelector(':scope > [data-icon="check"]')).not.toBeNull();
        expect(button.textContent!.trim()).toContain(en['shell.copied']);
        const region = page.querySelector('[aria-live="polite"]')!;
        expect(region.textContent).toBe(en['shell.copiedToClipboard']);

        await vi.advanceTimersByTimeAsync(COPIED_FOR_MS);
        expect(button.classList.contains('cb-copied')).toBe(false);
        expect(button.textContent!.trim()).toBe(en['view.copy']);
        expect(button.querySelector('[data-icon="copy"]')).not.toBeNull();
        expect(region.textContent).toBe('');
    });

    it('restores the copy button after a tab switch during the copied state', async () => {
        const writeText = vi.fn(async (_text: string) => {});
        Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
        const page = await show(THREE);
        page.querySelector<HTMLButtonElement>('#cb-item-tab-1')!.click();
        await flush();

        vi.useFakeTimers();
        page.querySelector<HTMLButtonElement>('[data-item-copy]')!.click();
        await vi.advanceTimersByTimeAsync(0);
        expect(writeText).toHaveBeenLastCalledWith(THREE[1]!.text);

        // Switch to another text item while "Copied" is shown.
        page.querySelector<HTMLButtonElement>('#cb-item-tab-2')!.click();
        await vi.advanceTimersByTimeAsync(0);
        await vi.advanceTimersByTimeAsync(COPIED_FOR_MS);

        const buttons = page.querySelectorAll<HTMLButtonElement>('[data-item-copy]');
        expect(buttons).toHaveLength(1);
        const button = buttons[0]!;
        expect(button.classList.contains('cb-copied')).toBe(false);
        expect(button.textContent!.trim()).toBe(en['view.copy']);
        expect(button.querySelectorAll('svg')).toHaveLength(1);
        expect(button.querySelector('[data-icon="copy"]')).not.toBeNull();
        expect(button.querySelector('[data-icon="check"]')).toBeNull();
        expect(button.querySelector('[data-copy-parked]')).toBeNull();

        // Copy now puts the newly selected item's text on the clipboard.
        button.click();
        await vi.advanceTimersByTimeAsync(0);
        expect(writeText).toHaveBeenLastCalledWith(THREE[2]!.text);
        expect(button.classList.contains('cb-copied')).toBe(true);
    });

    it('lists every item under "All N" when a tab does not fit', async () => {
        stubLayout(300);
        const items = Array.from({ length: 16 }, (_, i) => ({
            name: `file-${i + 1}.txt`,
            text: 'x'.repeat(i + 1),
        }));
        const page = await show(items);
        await flush();

        const all = page.querySelector<HTMLButtonElement>('[data-items-all]')!;
        expect(all.hidden).toBe(false);
        expect(all.textContent!.trim()).toBe('All 16');
        const menu = page.querySelector<HTMLElement>('#cb-all-menu')!;
        expect(menu.hidden).toBe(true);

        all.click();
        await flush();
        expect(menu.hidden).toBe(false);
        expect(all.getAttribute('aria-expanded')).toBe('true');
        const rows = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
        expect(rows.map((row) => row.querySelector('.cb-itemtab-name')!.textContent)).toEqual(
            items.map((item) => item.name),
        );
        expect(texts(rows.slice(0, 2))).toEqual(['Text file-1.txt 1 B', 'Text file-2.txt 2 B']);
        expect(rows.map((row) => row.getAttribute('aria-checked'))).toEqual(
            items.map((_, i) => String(i === 0)),
        );
        expect(document.activeElement).toBe(rows[0]);

        // Arrow keys move within the list; Escape closes it and returns to the button.
        rows[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
        expect(document.activeElement).toBe(rows[1]);
        rows[1]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true }));
        expect(document.activeElement).toBe(rows[0]);
        rows[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
        await flush();
        expect(menu.hidden).toBe(true);
        expect(document.activeElement).toBe(all);

        // Choosing a row selects it, keeps it visible as a tab and closes the list.
        all.click();
        await flush();
        rows[11]!.click();
        await flush();
        expect(menu.hidden).toBe(true);
        const tab = page.querySelector<HTMLButtonElement>('#cb-item-tab-11')!;
        expect(tab.getAttribute('aria-selected')).toBe('true');
        expect(tab.hidden).toBe(false);
        expect(document.activeElement).toBe(tab);
        expect(rows[11]!.getAttribute('aria-checked')).toBe('true');
        expect(page.querySelector('.cb-item-panel .cb-pre')!.textContent).toBe('x'.repeat(12));

        // The list is the same however many tabs fit.
        const narrow = texts(rows);
        stubLayout(5000);
        page.querySelector<HTMLButtonElement>('#cb-item-tab-0')!.click();
        await flush();
        expect(all.hidden).toBe(true);
        expect(texts(menu.querySelectorAll('[role="menuitemradio"]'))).toEqual(narrow);
    });

    it('offers no "All N" button when every tab fits', async () => {
        stubLayout(5000);
        const page = await show(THREE);
        await flush();
        expect(page.querySelector<HTMLButtonElement>('[data-items-all]')!.hidden).toBe(true);
        expect(
            [...page.querySelectorAll<HTMLButtonElement>('[role="tab"]')].every(
                (tab) => !tab.hidden,
            ),
        ).toBe(true);
    });

    it('steps to the previous and next item below the content', async () => {
        const scrollBy = stubLayout(5000);
        const page = await show(THREE);
        const nav = page.querySelector('[data-sheet] .cb-item-nav')!;
        const panel = page.querySelector('.cb-item-panel')!;
        expect(panel.compareDocumentPosition(nav) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

        const previous = nav.querySelector<HTMLButtonElement>('[data-item-previous]')!;
        const next = nav.querySelector<HTMLButtonElement>('[data-item-next]')!;
        expect(previous.getAttribute('aria-label')).toBe(en['items.previous']);
        expect(next.getAttribute('aria-label')).toBe(en['items.next']);
        expect(previous.disabled).toBe(true);
        expect(next.disabled).toBe(false);
        expect(next.textContent!.trim()).toBe('main.rs');
        expect(nav.querySelector('.cb-item-nav-position')!.textContent!.trim()).toBe('1 of 3');

        next.click();
        await flush();
        expect(page.querySelector('#cb-item-tab-1')!.getAttribute('aria-selected')).toBe('true');
        expect(nav.querySelector('.cb-item-nav-position')!.textContent!.trim()).toBe('2 of 3');
        expect(previous.textContent!.trim()).toBe('notes.md');
        expect(next.textContent!.trim()).toBe('todo.txt');
        expect(previous.disabled).toBe(false);
        // Scrolled past the item's start: back to it, just below the sticky bar.
        expect(scrollBy).toHaveBeenCalledWith({ top: -548 });

        next.click();
        await flush();
        expect(next.disabled).toBe(true);
        expect(nav.querySelector('.cb-item-nav-position')!.textContent!.trim()).toBe('3 of 3');

        previous.click();
        await flush();
        expect(page.querySelector('#cb-item-tab-1')!.getAttribute('aria-selected')).toBe('true');
    });

    it('numbers the lines of code and copies the code without them', async () => {
        const writeText = vi.fn(async (_text: string) => {});
        Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
        const code = 'fn main() {\n    /* a\n       b */\n}\n';
        const page = await show([
            { name: 'main.rs', text: code, presentation: 'code', language: 'rust' },
        ]);

        expect(page.querySelector('.cb-code-gutter')!.textContent).toBe('1\n2\n3\n4');
        expect(page.querySelector('.cb-code-block code')!.textContent).toBe(code);
        page.querySelector<HTMLButtonElement>('[data-item-copy]')!.click();
        await flush();
        expect(writeText).toHaveBeenCalledExactlyOnceWith(code);
    });
});
