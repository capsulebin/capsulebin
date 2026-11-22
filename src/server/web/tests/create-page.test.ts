/**
 * The create page's item tabs wired to the real HTML: adding items through the
 * menu, names, type changes that would lose content, removal, order, and what
 * ends up in the capsule.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
    derivePasswordKeyFor,
    MAX_ITEM_BYTES,
    MAX_ITEMS,
    openPaste,
    parseLink,
    PasteApi,
    type PublicHeader,
} from '@capsulebin/core';
import { createFakeServer, type FakeServer } from '../../../core/tests/fake-server';
import { CREATION_TOKEN_KEY, type CreationMode } from '../src/creation-token';
import de from '../src/i18n/de.json';
import en from '../src/i18n/en.json';
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

/** Waits for the sealed page and returns the share link shown on it. */
async function sealedShareLink(): Promise<string> {
    await vi.waitFor(() => expect(document.querySelector('[data-share-link]')).not.toBeNull(), {
        timeout: 15_000,
    });
    return el('share-link').textContent!;
}

async function loadPage(lang: 'en' | 'de' = 'en', mode: CreationMode = 'public'): Promise<void> {
    const shell = servedShell('index', lang).replace(
        '<meta name="capsulebin-creation" content="public"',
        `<meta name="capsulebin-creation" content="${mode}"`,
    );
    expect(shell).toContain(`<meta name="capsulebin-creation" content="${mode}"`);
    document.documentElement.innerHTML = new DOMParser().parseFromString(
        shell,
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

describe('create page password', () => {
    const CATALOGUES = { en, de };
    let server: FakeServer;

    beforeEach(() => {
        server = createFakeServer(window.location.origin);
        vi.stubGlobal('fetch', server.fetch);
    });

    function input<T extends HTMLElement>(id: string): T {
        return document.getElementById(id) as T;
    }

    function submit(): void {
        el<HTMLFormElement>('create-form').dispatchEvent(
            new Event('submit', { cancelable: true, bubbles: true }),
        );
    }

    function creates(): number {
        return server.requests.filter(
            (r) => r.method === 'POST' && r.url.endsWith('/api/v1/pastes'),
        ).length;
    }

    /** Submits and waits for the sealed page; returns the share link. */
    async function create(): Promise<string> {
        submit();
        return sealedShareLink();
    }

    async function header(shareUrl: string): Promise<PublicHeader> {
        const { id } = parseLink(shareUrl, 'share');
        const encoded = server.pastes.get(id)!.publicHeader;
        return JSON.parse(Buffer.from(encoded, 'base64url').toString('utf8')) as PublicHeader;
    }

    async function opensWith(shareUrl: string, password: string): Promise<boolean> {
        const link = parseLink(shareUrl, 'share');
        const outcome = await openPaste(new PasteApi(link.base), link.id, link.secret, {
            passwordKey: (h) => derivePasswordKeyFor(h, password),
        });
        return outcome.kind === 'success';
    }

    for (const lang of ['en', 'de'] as const) {
        it.each([' ', '   ', '\t'])(
            `refuses a password of only whitespace (%j) in ${lang} and keeps the form`,
            async (blank) => {
                await loadPage(lang);
                type('keep me');
                input<HTMLInputElement>('create-title').value = 'My title';
                input<HTMLSelectElement>('create-expiry').value = '86400';
                input<HTMLInputElement>('create-burn').checked = true;
                input<HTMLInputElement>('create-password').value = blank;
                submit();
                await new Promise((resolve) => setTimeout(resolve, 50));

                expect(creates()).toBe(0);
                expect(server.requests).toHaveLength(0);
                // Still the form: neither the encrypting nor the error view.
                expect(el('create-form')).not.toBeNull();
                expect(document.querySelector('[data-hexagon]')).toBeNull();
                expect(document.querySelector('[data-error-message]')).toBeNull();

                const message = el('password-error');
                expect(message.hidden).toBe(false);
                expect(message.textContent).toBe(CATALOGUES[lang]['create.passwordBlank']);
                expect(document.activeElement).toBe(input('create-password'));
                expect(input('create-password').getAttribute('aria-invalid')).toBe('true');

                expect(el<HTMLTextAreaElement>('item-text').value).toBe('keep me');
                expect(input<HTMLInputElement>('create-title').value).toBe('My title');
                expect(input<HTMLSelectElement>('create-expiry').value).toBe('86400');
                expect(input<HTMLInputElement>('create-burn').checked).toBe(true);
                expect(input<HTMLInputElement>('create-password').value).toBe(blank);
            },
        );
    }

    it('shows no password message before a submit', () => {
        expect(el('password-error').hidden).toBe(true);
        expect(input('create-password').hasAttribute('aria-invalid')).toBe(false);
    });

    it('creates a capsule without a password from an empty field', async () => {
        type('no password');
        const share = await create();
        expect(creates()).toBe(1);
        expect((await header(share)).password).toBeUndefined();
    });

    it('passes a password with spaces around it on unchanged', async () => {
        type('protected');
        input<HTMLInputElement>('create-password').value = ' secret ';
        const share = await create();
        expect((await header(share)).password).toBeDefined();
        expect(await opensWith(share, 'secret')).toBe(false);
        expect(await opensWith(share, ' secret ')).toBe(true);
    }, 30_000);

    it('creates the capsule once a refused password is corrected', async () => {
        type('second try');
        input<HTMLInputElement>('create-password').value = '  ';
        submit();
        expect(el('password-error').hidden).toBe(false);
        expect(creates()).toBe(0);

        input<HTMLInputElement>('create-password').value = 'correct horse';
        submit();
        // The form gives way to the encrypting view, without the message.
        expect(document.querySelector('[data-password-error]:not([hidden])')).toBeNull();
        const share = await sealedShareLink();
        expect(creates()).toBe(1);
        expect(await opensWith(share, 'correct horse')).toBe(true);
    }, 30_000);
});

describe('create page creation modes', () => {
    const TOKEN = 'good-token_123';
    let server: FakeServer;

    beforeEach(() => {
        server = createFakeServer(window.location.origin);
        server.creationToken = TOKEN;
        vi.stubGlobal('fetch', server.fetch);
    });

    afterEach(() => {
        localStorage.clear();
        history.replaceState(null, '', '/');
    });

    function query<T extends HTMLElement>(attr: string): T | null {
        return document.querySelector<T>(`[data-${attr}]`);
    }

    function field<T extends HTMLElement>(id: string): T {
        return document.getElementById(id) as T;
    }

    function enterToken(value: string, remember = true): void {
        el<HTMLInputElement>('token-input').value = value;
        el<HTMLInputElement>('token-remember').checked = remember;
        el<HTMLFormElement>('token-form').dispatchEvent(
            new Event('submit', { cancelable: true, bubbles: true }),
        );
    }

    function submit(): void {
        el<HTMLFormElement>('create-form').dispatchEvent(
            new Event('submit', { cancelable: true, bubbles: true }),
        );
    }

    function creates(): FakeServer['requests'] {
        return server.requests.filter(
            (r) => r.method === 'POST' && r.url.endsWith('/api/v1/pastes'),
        );
    }

    function stored(): string | null {
        return localStorage.getItem(CREATION_TOKEN_KEY);
    }

    function done(): Promise<string> {
        return sealedShareLink();
    }

    async function failed(): Promise<string> {
        await vi.waitFor(() => expect(query('error-message')).not.toBeNull(), {
            timeout: 15_000,
        });
        return el('error-message').textContent!;
    }

    describe('token mode without a token', () => {
        beforeEach(async () => {
            await loadPage('en', 'token');
        });

        it('shows the notice and the token field instead of the editor', () => {
            expect(query('create-form')).toBeNull();
            expect(document.body.textContent).toContain(en['create.token.required']);
            expect(el<HTMLInputElement>('token-input')).not.toBeNull();
            expect(el<HTMLInputElement>('token-remember').checked).toBe(true);
            expect(el('token-error').hidden).toBe(true);
        });

        it.each([
            ['empty', ''],
            ['blank', '   '],
            ['with a space', 'ab cd'],
            ['too long', 'x'.repeat(1025)],
            ['non-ASCII', 'tök'],
        ])('refuses a token that is %s inline and does not use it', (_case, value) => {
            enterToken(value);
            expect(query('create-form')).toBeNull();
            expect(el('token-error').hidden).toBe(false);
            expect(el('token-error').textContent).toBe(en['create.token.invalid']);
            expect(el('token-input').getAttribute('aria-invalid')).toBe('true');
            expect(stored()).toBeNull();
        });

        // AC5: the upper bound itself is a usable token.
        it('accepts a token of exactly 1024 visible ASCII characters', async () => {
            const long = '~'.repeat(1024);
            server.creationToken = long;
            enterToken(long);
            expect(query('token-input')).toBeNull();
            expect(query('create-form')).not.toBeNull();
            expect(stored()).toBe(long);
            type('long token');
            submit();
            await done();
            expect(creates()[0]!.headers.get('Authorization')).toBe(`Bearer ${long}`);
        });

        it('shows the editor and the stored line for a token, trimmed', () => {
            enterToken(`  ${TOKEN} `);
            expect(query('create-form')).not.toBeNull();
            expect(query('token-input')).toBeNull();
            expect(el('token-line').hidden).toBe(false);
            expect(el('token-line-text').textContent).toBe(en['create.token.stored']);
            expect(stored()).toBe(TOKEN);
        });

        it('sends the token as the bearer of the create request only', async () => {
            enterToken(TOKEN);
            type('with token');
            submit();
            await done();
            const [create] = creates();
            expect(create!.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
            for (const request of server.requests) {
                expect(request.url).not.toContain(TOKEN);
                expect(request.body).not.toContain(TOKEN);
            }
        });

        it('holds a token it should not remember for this page only', async () => {
            enterToken(TOKEN, false);
            expect(el('token-line-text').textContent).toBe(en['create.token.pageOnly']);
            type('page only');
            submit();
            await done();
            expect(creates()[0]!.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
            expect(stored()).toBeNull();

            await loadPage('en', 'token');
            expect(query('create-form')).toBeNull();
            expect(query('token-input')).not.toBeNull();
        });
    });

    it('asks for the token in German', async () => {
        await loadPage('de', 'token');
        const main = document.querySelector('main')!.textContent!;
        expect(main).toContain(de['create.token.required']);
        expect(main).toContain(de['create.token.label']);
        expect(main).toContain(de['create.token.remember']);
        expect(el('token-submit').textContent).toBe(de['create.token.continue']);
        enterToken('x y');
        expect(el('token-error').textContent).toBe(de['create.token.invalid']);
        enterToken(TOKEN);
        expect(el('token-line-text').textContent).toBe(de['create.token.stored']);
        expect(el('token-forget').textContent!.trim()).toBe(de['create.token.forget']);
    });

    it('opens with the editor for a stored token, and forgets it', async () => {
        localStorage.setItem(CREATION_TOKEN_KEY, TOKEN);
        await loadPage('en', 'token');
        expect(query('create-form')).not.toBeNull();
        expect(el('token-line-text').textContent).toBe(en['create.token.stored']);

        type('kept draft');
        el<HTMLButtonElement>('token-forget').click();
        expect(stored()).toBeNull();
        expect(query('create-form')).toBeNull();
        expect(query('token-input')).not.toBeNull();

        enterToken(TOKEN, false);
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('kept draft');
        el<HTMLButtonElement>('token-forget').click();
        expect(query('token-input')).not.toBeNull();
        expect(stored()).toBeNull();
    });

    describe('when localStorage throws', () => {
        const descriptor = Object.getOwnPropertyDescriptor(window, 'localStorage')!;

        afterEach(() => {
            Object.defineProperty(window, 'localStorage', descriptor);
        });

        it('still creates with a token for this page', async () => {
            Object.defineProperty(window, 'localStorage', {
                configurable: true,
                get() {
                    throw new DOMException('blocked', 'SecurityError');
                },
            });
            await loadPage('en', 'token');
            enterToken(TOKEN);
            expect(el('token-line-text').textContent).toBe(en['create.token.pageOnly']);
            type('no storage');
            submit();
            await done();
            expect(creates()[0]!.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
        });

        it('uses an invitation for this page only, without the stored confirmation', async () => {
            history.replaceState(null, '', `/#token=${TOKEN}`);
            Object.defineProperty(window, 'localStorage', {
                configurable: true,
                get() {
                    throw new DOMException('blocked', 'SecurityError');
                },
            });
            await loadPage('en', 'token');
            expect(location.hash).toBe('');
            expect(query('create-form')).not.toBeNull();
            expect(el('token-line-text').textContent).toBe(en['create.token.pageOnly']);
            expect(el('token-notice').hidden).toBe(true);
            expect(document.body.textContent).not.toContain(en['create.token.accepted']);
            type('invited, no storage');
            submit();
            await done();
            expect(creates()[0]!.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
        });
    });

    describe('public mode', () => {
        it('shows no token UI and sends no Authorization header, even with a stored token', async () => {
            server.creationToken = null;
            localStorage.setItem(CREATION_TOKEN_KEY, TOKEN);
            await loadPage('en', 'public');
            expect(el('token-line').hidden).toBe(true);
            expect(query('token-input')).toBeNull();
            type('public');
            submit();
            await done();
            expect(creates()[0]!.headers.has('Authorization')).toBe(false);
        });
    });

    describe('disabled mode', () => {
        it('shows only the notice, no form and no token field', async () => {
            await loadPage('en', 'disabled');
            expect(query('create-form')).toBeNull();
            expect(query('token-input')).toBeNull();
            expect(query('token-form')).toBeNull();
            expect(document.querySelector('main')!.textContent).toContain(
                en['create.disabled.text'],
            );
        });

        it('in German too', async () => {
            await loadPage('de', 'disabled');
            expect(document.querySelector('main')!.textContent).toContain(
                de['create.disabled.text'],
            );
        });
    });

    describe('invitation link', () => {
        it.each(['en', 'de'] as const)(
            'stores the token, leaves the address and confirms (%s)',
            async (lang) => {
                const path = lang === 'en' ? '/' : '/de/';
                history.replaceState(null, '', `${path}#token=${TOKEN}`);
                const length = history.length;
                await loadPage(lang, 'token');
                expect(location.hash).toBe('');
                expect(location.href).toBe(`${location.origin}${path}`);
                expect(history.length).toBe(length);
                expect(stored()).toBe(TOKEN);
                expect(el('token-notice').hidden).toBe(false);
                expect(el('token-notice').textContent).toBe(
                    { en, de }[lang]['create.token.accepted'],
                );
                expect(query('create-form')).not.toBeNull();
            },
        );

        it('stores nothing for an unusable value and says so', async () => {
            history.replaceState(null, '', `/#token=${'x'.repeat(1025)}`);
            await loadPage('en', 'token');
            expect(location.hash).toBe('');
            expect(stored()).toBeNull();
            expect(el('token-notice').textContent).toBe(en['create.token.invitationInvalid']);
            expect(query('create-form')).toBeNull();
            expect(query('token-input')).not.toBeNull();
        });

        it('leaves any other fragment alone', async () => {
            history.replaceState(null, '', '/#section');
            await loadPage('en', 'token');
            expect(location.hash).toBe('#section');
            expect(el('token-notice').hidden).toBe(true);
        });

        it.each(['public', 'disabled'] as const)(
            'is removed and stores nothing in %s mode',
            async (mode) => {
                history.replaceState(null, '', `/#token=${TOKEN}`);
                await loadPage('en', mode);
                expect(location.hash).toBe('');
                expect(stored()).toBeNull();
                expect(document.querySelector('[data-token-notice]:not([hidden])')).toBeNull();
            },
        );
    });

    /** Two items (text and code), the first active, title, expiry, burn and password. */
    function writeDraft(): void {
        type('first item');
        add('code');
        type('fn main() {}');
        const lang = el<HTMLSelectElement>('item-lang');
        lang.value = 'rust';
        lang.dispatchEvent(new Event('change'));
        document.querySelector<HTMLElement>('[role="tab"]')!.click();
        field<HTMLInputElement>('create-title').value = 'Draft title';
        field<HTMLSelectElement>('create-expiry').value = '3600';
        field<HTMLInputElement>('create-burn').checked = true;
        field<HTMLInputElement>('create-password').value = 'pass phrase';
    }

    /** The draft of {@link writeDraft}, after the items in `more`. */
    function expectDraft(...more: string[]): void {
        expect(tabs()).toEqual(['text.txt *', 'snippet.rs', ...more]);
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('first item');
        expect(field<HTMLInputElement>('create-title').value).toBe('Draft title');
        expect(field<HTMLSelectElement>('create-expiry').value).toBe('3600');
        expect(field<HTMLInputElement>('create-burn').checked).toBe(true);
        expect(field<HTMLInputElement>('create-password').value).toBe('pass phrase');
        document.querySelectorAll<HTMLElement>('[role="tab"]')[1]!.click();
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('fn main() {}');
        expect(el<HTMLSelectElement>('item-type').value).toBe('code');
        expect(el<HTMLSelectElement>('item-lang').value).toBe('rust');
    }

    it('asks again for a refused token and sends the same sealed capsule', async () => {
        localStorage.setItem(CREATION_TOKEN_KEY, 'wrong-token');
        await loadPage('en', 'token');
        writeDraft();
        submit();

        expect(await failed()).toBe(en['create.token.refused']);
        expect(stored()).toBeNull();
        expect(el<HTMLInputElement>('token-remember').checked).toBe(true);
        expect(el('token-submit').textContent).toBe(en['create.token.create']);

        enterToken('still-wrong');
        expect(await failed()).toBe(en['create.token.refused']);
        expect(query('token-input')).not.toBeNull();

        enterToken(TOKEN);
        const share = await done();
        const bodies = creates().map((r) => r.body);
        expect(bodies).toHaveLength(3);
        // No new encryption: the same request, the same id.
        expect(new Set(bodies).size).toBe(1);
        expect(parseLink(share, 'share').id).toBe(JSON.parse(bodies[0]!).id);
        expect(creates().map((r) => r.headers.get('Authorization'))).toEqual([
            'Bearer wrong-token',
            'Bearer still-wrong',
            `Bearer ${TOKEN}`,
        ]);
        expect(stored()).toBe(TOKEN);
    }, 60_000);

    describe('"Back to editing"', () => {
        function problem(status: number, code: string): Response {
            return new Response(JSON.stringify({ title: code, status, code }), {
                status,
                headers: { 'Content-Type': 'application/problem+json' },
            });
        }

        it.each([
            ['401', () => Promise.resolve(problem(401, 'unauthorized'))],
            ['413', () => Promise.resolve(problem(413, 'payload_too_large'))],
            ['429', () => Promise.resolve(problem(429, 'rate_limited'))],
            ['a network error', () => Promise.reject(new TypeError('Failed to fetch'))],
        ])(
            'restores the draft as submitted after %s',
            async (_case, answer) => {
                localStorage.setItem(CREATION_TOKEN_KEY, TOKEN);
                await loadPage('en', 'token');
                vi.stubGlobal('fetch', answer);
                writeDraft();
                const file = new File([new Uint8Array([1, 2, 3])], 'data.bin');
                const input = el<HTMLInputElement>('item-file');
                Object.defineProperty(input, 'files', { configurable: true, value: [file] });
                input.dispatchEvent(new Event('change'));
                await vi.waitFor(() => expect(tabs()).toHaveLength(3));
                await vi.waitFor(() => expect(el('item-drop-label').textContent).toContain('3 B'));
                document.querySelector<HTMLElement>('[role="tab"]')!.click();
                submit();

                await failed();
                el<HTMLButtonElement>('retry-btn').click();
                expect(query('create-form')).not.toBeNull();
                expect(tabs()).toEqual(['text.txt *', 'snippet.rs', 'data.bin']);
                document.querySelectorAll<HTMLElement>('[role="tab"]')[2]!.click();
                expect(el<HTMLSelectElement>('item-type').value).toBe('download');
                expect(el('item-drop-label').textContent).toBe('data.bin (3 B)');
                document.querySelector<HTMLElement>('[role="tab"]')!.click();
                expectDraft('data.bin');
            },
            30_000,
        );

        // AC10: "any error" includes public mode, and edited item names survive.
        it('restores renamed items in public mode after a 413', async () => {
            await loadPage('en', 'public');
            vi.stubGlobal('fetch', () => Promise.resolve(problem(413, 'payload_too_large')));
            type('alpha');
            const name = el<HTMLInputElement>('item-name');
            name.value = 'notes.md';
            name.dispatchEvent(new Event('input'));
            add('plain');
            type('beta');
            field<HTMLInputElement>('create-title').value = 'Public draft';
            submit();

            await failed();
            el<HTMLButtonElement>('retry-btn').click();
            expect(query('create-form')).not.toBeNull();
            expect(tabs()).toHaveLength(2);
            expect(tabs()[0]).toBe('notes.md');
            expect(tabs()[1]).toMatch(/ \*$/);
            expect(el<HTMLTextAreaElement>('item-text').value).toBe('beta');
            expect(field<HTMLInputElement>('create-title').value).toBe('Public draft');
            document.querySelector<HTMLElement>('[role="tab"]')!.click();
            expect(el<HTMLInputElement>('item-name').value).toBe('notes.md');
            expect(el<HTMLTextAreaElement>('item-text').value).toBe('alpha');
        }, 30_000);

        // AC10: the active item is restored as submitted, also when it is not the first.
        it('restores a second active item after a failed create', async () => {
            localStorage.setItem(CREATION_TOKEN_KEY, TOKEN);
            await loadPage('en', 'token');
            vi.stubGlobal('fetch', () => Promise.resolve(problem(429, 'rate_limited')));
            writeDraft();
            document.querySelectorAll<HTMLElement>('[role="tab"]')[1]!.click();
            expect(el<HTMLTextAreaElement>('item-text').value).toBe('fn main() {}');
            submit();

            await failed();
            el<HTMLButtonElement>('retry-btn').click();
            expect(query('create-form')).not.toBeNull();
            const selected = [...document.querySelectorAll<HTMLElement>('[role="tab"]')].map(
                (tab) => tab.getAttribute('aria-selected'),
            );
            expect(selected).toEqual(['false', 'true']);
            expect(el<HTMLTextAreaElement>('item-text').value).toBe('fn main() {}');
            expect(el<HTMLSelectElement>('item-type').value).toBe('code');
            expect(el<HTMLSelectElement>('item-lang').value).toBe('rust');
        }, 30_000);
    });
});

describe('create page layout', () => {
    it('starts with the editor: a screen-reader-only heading, no visible heading or lede', () => {
        const form = el('create-form');
        const headings = [...document.querySelectorAll('main h1')];
        expect(headings).toHaveLength(1);
        expect(headings[0]!.classList.contains('cb-sr-only')).toBe(true);
        expect(headings[0]!.textContent).toBe(en['create.heading']);
        expect(document.querySelector('main .cb-lede')).toBeNull();
        for (const node of document.querySelectorAll('main > *')) {
            if (node === form) break;
            if (node.matches('[hidden], .cb-sr-only')) continue;
            // Only the token notice and line may stand before the form, and they are hidden here.
            throw new Error(`visible element before the form: ${node.outerHTML}`);
        }
    });

    it('keeps the title input borderless but labelled', () => {
        const title = document.getElementById('create-title')!;
        expect(title.classList.contains('cb-create-title')).toBe(true);
        expect(document.querySelector('label[for="create-title"]')!.textContent).toBe(
            en['create.titleLabel'],
        );
    });

    it('puts tabs, type, name, language, text and size into one editor frame', () => {
        const editor = el('editor');
        expect(editor.classList.contains('cb-editor-frame')).toBe(true);
        for (const part of [
            '[data-item-tabs]',
            '#create-item-type',
            '#create-item-name',
            '#create-item-lang',
            '[data-item-text]',
            '[data-size-meter]',
            '[data-size-bar]',
        ]) {
            expect(editor.querySelector(part), part).not.toBeNull();
        }
    });

    it('shows the remove action as an icon with a label narrow screens hide', () => {
        type('a');
        add('plain');
        const remove = el<HTMLButtonElement>('item-remove');
        expect(remove.hidden).toBe(false);
        expect(remove.querySelector('svg[data-icon="remove"]')).not.toBeNull();
        expect(remove.querySelector('.cb-bar-label')!.textContent).toBe(en['create.remove']);
    });

    it('puts expiry, burn and password in one options row, each with its label', () => {
        const fields = [...document.querySelectorAll('.cb-create-options > .cb-create-field')];
        expect(fields).toHaveLength(3);
        expect(fields[0]!.querySelector('label[for="create-expiry"]')!.textContent).toBe(
            en['create.expires'],
        );
        expect(fields[0]!.querySelector('#create-expiry')).not.toBeNull();
        const burn = fields[1]!;
        expect(burn.getAttribute('role')).toBe('group');
        expect(document.getElementById(burn.getAttribute('aria-labelledby')!)!.textContent).toBe(
            en['create.burnLabel'],
        );
        expect(burn.querySelector('#create-burn')).not.toBeNull();
        expect(fields[2]!.querySelector('label[for="create-password"]')!.textContent).toBe(
            en['create.password'],
        );
        expect(fields[2]!.querySelector('#create-password')).not.toBeNull();
    });

    it('shows the password hint only while the password field has content', () => {
        const note = el('password-note');
        const password = document.getElementById('create-password') as HTMLInputElement;
        expect(note.textContent!.trim()).toBe(en['create.passwordNote']);
        expect(note.hidden).toBe(true);
        password.value = 's';
        password.dispatchEvent(new Event('input', { bubbles: true }));
        expect(note.hidden).toBe(false);
        password.value = '';
        password.dispatchEvent(new Event('input', { bubbles: true }));
        expect(note.hidden).toBe(true);
    });

    it('puts the trust line next to the submit button', () => {
        const row = el('create-submit').parentElement!;
        expect(row.classList.contains('cb-create-submit')).toBe(true);
        expect(row.querySelector('[data-trust-line]')!.textContent).toBe(en['create.lede']);
    });
});

describe('create page size bar', () => {
    function fill(): string {
        return el('size-fill').getAttribute('width')!;
    }

    it('fills its share of the limit, as an attribute and never as a style', () => {
        expect(fill()).toBe('0.0%');
        type('x'.repeat(Math.round(MAX_ITEM_BYTES / 4)));
        expect(fill()).toBe('25.0%');
        expect(el('size-meter').textContent).toMatch(/ of 9\.9 MiB$/);
        expect(el('size-bar').getAttribute('aria-hidden')).toBe('true');
        expect(document.querySelector('[data-editor] [style]')).toBeNull();
    });

    it('turns to the danger colour over the limit and stays full', () => {
        type('x'.repeat(MAX_ITEM_BYTES + 1));
        expect(fill()).toBe('100.0%');
        expect(el('size-bar').classList.contains('cb-size-bar-over')).toBe(true);
        expect(el('size-meter').classList.contains('cb-size-meter-over')).toBe(true);
        expect(el('size-meter').textContent).toContain('over the limit by 1 B');
        expect(el<HTMLButtonElement>('create-submit').disabled).toBe(true);

        type('x');
        expect(el('size-bar').classList.contains('cb-size-bar-over')).toBe(false);
    });
});

describe('create page "+ Add item"', () => {
    function fillUp(count: number): void {
        type('0');
        for (let i = 1; i < count; i++) {
            add('plain');
            type(String(i));
        }
    }

    it('stays right of "All N"', () => {
        const more = el('more');
        const addButton = el('item-add');
        expect(more.compareDocumentPosition(addButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
            Node.DOCUMENT_POSITION_FOLLOWING,
        );
    });

    it('is disabled and reads "16 of 16" at the limit, and comes back after a removal', () => {
        fillUp(MAX_ITEMS - 1);
        const addButton = el<HTMLButtonElement>('item-add');
        expect(addButton.disabled).toBe(false);
        expect(addButton.textContent).toBe(en['create.addItem']);

        add('plain');
        expect(tabs()).toHaveLength(MAX_ITEMS);
        expect(addButton.disabled).toBe(true);
        expect(addButton.textContent).toBe('16 of 16');
        addButton.click();
        expect(el('add-menu').hidden).toBe(true);

        document.querySelector<HTMLButtonElement>('[data-remove-id]')!.click();
        expect(addButton.disabled).toBe(false);
        expect(addButton.textContent).toBe(en['create.addItem']);
    });

    it('reads "16 von 16" in German', async () => {
        await loadPage('de');
        fillUp(MAX_ITEMS);
        expect(el('item-add').textContent).toBe('16 von 16');
    });
});

describe('create page with one item', () => {
    const bar = () => el('tabs-row');
    const inBar = () => bar().contains(el('item-add'));

    it('shows no tab bar and names the item only in its row', () => {
        expect(bar().hidden).toBe(true);
        expect(el('more').hidden).toBe(true);
        expect(el<HTMLInputElement>('item-name').value).toBe('text.txt');
        // The only other place holding the name is the hidden bar.
        const named = [...el('editor').querySelectorAll('*')].filter(
            (node) => node.children.length === 0 && node.textContent === 'text.txt',
        );
        expect(named.every((node) => bar().contains(node))).toBe(true);
    });

    it('keeps the add button in reach, at the end of the item actions', () => {
        const addButton = el<HTMLButtonElement>('item-add');
        expect(inBar()).toBe(false);
        expect(el('item-actions').lastElementChild!.contains(addButton)).toBe(true);
        expect(el('editor-head').contains(addButton)).toBe(true);
        expect(addButton.disabled).toBe(false);

        addButton.click();
        expect(el('add-menu').hidden).toBe(false);
        expect(addButton.getAttribute('aria-expanded')).toBe('true');
    });

    it('brings the tab bar back with the second item and hides it after a removal', () => {
        type('one');
        add('plain');
        expect(bar().hidden).toBe(false);
        expect(inBar()).toBe(true);
        expect(bar().lastElementChild!.contains(el('item-add'))).toBe(true);
        expect(tabs()).toEqual(['text.txt', 'text-2.txt *']);

        el<HTMLButtonElement>('item-remove').click();
        expect(tabs()).toEqual(['text.txt *']);
        expect(bar().hidden).toBe(true);
        expect(inBar()).toBe(false);
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('one');
    });

    it('moves the focus to the item row when the tab close button leaves one item', () => {
        type('one');
        add('plain');
        const close = document.querySelector<HTMLButtonElement>('[data-remove-id]')!;
        close.focus();
        close.click();
        expect(tabs()).toEqual(['text-2.txt *']);
        expect(bar().hidden).toBe(true);
        expect(document.activeElement).toBe(el('item-type'));
    });

    it('moves the focus to the item row when "Remove" leaves one item', () => {
        type('one');
        add('plain');
        const remove = el<HTMLButtonElement>('item-remove');
        remove.focus();
        remove.click();
        expect(remove.hidden).toBe(true);
        expect(document.activeElement).toBe(el('item-type'));
    });

    it('keeps one bar-less item when the untouched item changes its kind', () => {
        add('markdown');
        expect(tabs()).toEqual(['notes.md *']);
        expect(bar().hidden).toBe(true);
        expect(inBar()).toBe(false);
    });

    it('shows the bar again for a restored draft with several items', async () => {
        vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
        type('one');
        add('plain');
        type('two');
        el<HTMLFormElement>('create-form').requestSubmit();
        await vi.waitFor(() => expect(document.querySelector('[data-retry-btn]')).not.toBeNull(), {
            timeout: 15_000,
        });
        el<HTMLButtonElement>('retry-btn').click();
        expect(bar().hidden).toBe(false);
        expect(inBar()).toBe(true);
        expect(tabs()).toEqual(['text.txt', 'text-2.txt *']);
    });
});

describe('create page "All N" list', () => {
    /** Width of the tab list; every tab and the "All N" button are 100 px wide. */
    let barWidth = 250;

    beforeEach(() => {
        barWidth = 250;
        vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
            this: HTMLElement,
        ) {
            return this.matches('[data-item-tabs]') ? barWidth : 0;
        });
        vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(
            () => ({ width: 100 }) as DOMRect,
        );
    });

    afterEach(() => {
        vi.restoreAllMocks();
    });

    /** Four items (text, Markdown, code, file name), the first one active. */
    function fourItems(): void {
        type('one');
        add('markdown');
        type('# two');
        add('code');
        type('three');
        add('plain');
        type('four');
        document.querySelector<HTMLElement>('[role="tab"]')!.click();
    }

    function open(): void {
        el<HTMLButtonElement>('more-btn').click();
    }

    function rows(): string[] {
        return [...el('more-menu').querySelectorAll<HTMLElement>('[data-more-id]')].map(
            (row) =>
                [...row.children].map((part) => part.textContent).join(' ') +
                (row.getAttribute('aria-checked') === 'true' ? ' *' : ''),
        );
    }

    function visibleTabs(): string[] {
        return [...document.querySelectorAll<HTMLElement>('[data-item-tabs] > *')]
            .filter((wrap) => !wrap.hidden)
            .map((wrap) => wrap.querySelector('.cb-itemtab-name')!.textContent!);
    }

    it('is absent while every tab fits', () => {
        barWidth = 1000;
        fourItems();
        expect(el('more').hidden).toBe(true);
    });

    it('lists every item in creation order with kind, name and a remove control', () => {
        fourItems();
        const button = el('more-btn');
        expect(el('more').hidden).toBe(false);
        expect(button.textContent).toBe('All 4');
        expect(button.querySelector('svg[data-icon="chevron-down"]')).not.toBeNull();
        expect(rows()).toEqual([
            'Text text.txt *',
            'MD notes.md',
            'Code snippet.txt',
            'Text text-2.txt',
        ]);
        const removes = [...el('more-menu').querySelectorAll('[data-menu-remove-id]')];
        expect(removes.map((remove) => remove.getAttribute('aria-label'))).toEqual([
            'Remove text.txt',
            'Remove notes.md',
            'Remove snippet.txt',
            'Remove text-2.txt',
        ]);
    });

    it('has the same contents at every width', () => {
        fourItems();
        const wide = el('more-menu').innerHTML;
        barWidth = 120;
        el<HTMLButtonElement>('item-name').dispatchEvent(new Event('input'));
        expect(visibleTabs()).toEqual(['text.txt']);
        expect(el('more-menu').innerHTML).toBe(wide);
    });

    it('selects the chosen item, keeps it visible as a tab and closes', () => {
        fourItems();
        expect(visibleTabs()).toEqual(['text.txt']);
        open();
        expect(el('more-menu').hidden).toBe(false);
        expect(el('more-btn').getAttribute('aria-expanded')).toBe('true');
        expect(document.activeElement!.getAttribute('aria-checked')).toBe('true');

        el('more-menu').querySelectorAll<HTMLElement>('[data-more-id]')[3]!.click();
        expect(el('more-menu').hidden).toBe(true);
        expect(tabs()[3]).toBe('text-2.txt *');
        expect(visibleTabs()).toEqual(['text-2.txt']);
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('four');
    });

    it('moves with the arrow keys and closes with Escape, back on the button', () => {
        fourItems();
        open();
        const keydown = (key: string) =>
            document.activeElement!.dispatchEvent(
                new KeyboardEvent('keydown', { key, bubbles: true }),
            );
        const buttons = [...el('more-menu').querySelectorAll('button')];
        expect(document.activeElement).toBe(buttons[0]);
        keydown('ArrowDown');
        expect(document.activeElement).toBe(buttons[1]);
        keydown('ArrowUp');
        expect(document.activeElement).toBe(buttons[0]);

        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
        expect(el('more-menu').hidden).toBe(true);
        expect(document.activeElement).toBe(el('more-btn'));
    });

    it('removes an item with its remove control and keeps the draft logic', () => {
        fourItems();
        open();
        el('more-menu').querySelector<HTMLButtonElement>('[aria-label="Remove notes.md"]')!.click();
        expect(tabs()).toEqual(['text.txt *', 'snippet.txt', 'text-2.txt']);
        expect(rows()).toEqual(['Text text.txt *', 'Code snippet.txt', 'Text text-2.txt']);
        // Still open: a click inside the menu does not close it.
        expect(el('more-menu').hidden).toBe(false);
        expect(document.activeElement!.getAttribute('aria-checked')).toBe('true');
    });

    it('moves the focus to the item row when a removal from the list leaves one item', () => {
        barWidth = 120;
        type('one');
        add('markdown');
        expect(el('more-btn').textContent).toBe('All 2');
        open();
        const remove = el('more-menu').querySelector<HTMLButtonElement>(
            '[aria-label="Remove notes.md"]',
        )!;
        remove.focus();
        remove.click();
        expect(tabs()).toEqual(['text.txt *']);
        expect(el('tabs-row').hidden).toBe(true);
        expect(el('more-menu').hidden).toBe(true);
        expect(document.activeElement).toBe(el('item-type'));
    });

    it('offers no remove control with one item', () => {
        barWidth = 50;
        expect(tabs()).toEqual(['text.txt *']);
        expect(el('more-menu').querySelectorAll('[data-more-id]')).toHaveLength(1);
        expect(el('more-menu').querySelector('[data-menu-remove-id]')).toBeNull();
    });
});

describe('create page states and sealed page', () => {
    let server: FakeServer;
    const writeText = vi.fn<(text: string) => Promise<void>>();

    beforeEach(() => {
        server = createFakeServer(window.location.origin);
        vi.stubGlobal('fetch', server.fetch);
        writeText.mockReset();
        writeText.mockResolvedValue(undefined);
        Object.defineProperty(navigator, 'clipboard', {
            configurable: true,
            value: { writeText },
        });
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    function submit(): void {
        el<HTMLFormElement>('create-form').dispatchEvent(
            new Event('submit', { cancelable: true, bubbles: true }),
        );
    }

    function field<T extends HTMLElement>(id: string): T {
        return document.getElementById(id) as T;
    }

    function chips(): string[] {
        return [...el('summary').querySelectorAll('li')].map((chip) => chip.textContent!);
    }

    it('shows the running hexagon while encrypting, without a spinner', async () => {
        type('content');
        submit();
        expect(document.querySelector('[data-hexagon="running"]')).not.toBeNull();
        expect(document.querySelector('main h1')!.textContent).toBe(en['create.encrypting']);
        expect(document.querySelector('.cb-spinner')).toBeNull();
        await sealedShareLink();
    });

    it('shows the empty hexagon on an error, and "Back to editing" still restores', async () => {
        vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')));
        type('content');
        submit();
        await vi.waitFor(() =>
            expect(document.querySelector('[data-error-message]')).not.toBeNull(),
        );
        expect(document.querySelector('[data-hexagon="empty"]')).not.toBeNull();
        expect(document.querySelector('main h1')!.textContent).toBe(en['create.error']);
        el<HTMLButtonElement>('retry-btn').click();
        expect(el<HTMLTextAreaElement>('item-text').value).toBe('content');
    });

    it('shows the share link as text with the key highlighted, and one filled button', async () => {
        type('content');
        submit();
        const share = await sealedShareLink();

        const main = document.querySelector('main')!;
        expect(main.querySelector('h1')!.textContent).toBe('Sealed.');
        expect(main.querySelector('[data-hexagon="sealed"]')).not.toBeNull();
        expect(main.querySelector('input')).toBeNull();

        const link = parseLink(share, 'share');
        expect(server.pastes.has(link.id)).toBe(true);
        const key = el('share-key');
        expect(el('share-link').contains(key)).toBe(true);
        expect(key.textContent).toBe(share.slice(share.indexOf('#')));
        expect(key.textContent).toMatch(/^#v1\./);
        expect(main.querySelector('.cb-link-panel')!.textContent).toContain(en['create.keyNote']);

        const filled = [...main.querySelectorAll('.cb-btn:not(.cb-btn-ghost)')];
        expect(filled).toHaveLength(1);
        expect(filled[0]!.textContent!.trim()).toBe(en['create.copyLink']);
        expect(filled[0]!.querySelector('svg[data-icon="copy"]')).not.toBeNull();
        expect(main.querySelector('a.cb-btn')).toBeNull();
        expect(main.querySelector(`a[href="/"]`)!.textContent).toBe(en['create.another']);
    });

    it('shows the delete link in a dashed row with its own copy button and note', async () => {
        type('content');
        submit();
        await sealedShareLink();
        const manage = el('delete-link').textContent!;
        const { id } = parseLink(manage, 'manage');
        expect(server.pastes.has(id)).toBe(true);

        const row = el('delete-row');
        expect(row.classList.contains('cb-sealed-delete')).toBe(true);
        expect(row.contains(el('delete-link'))).toBe(true);
        expect(row.textContent).toContain(en['create.deleteLinkNote']);
        const copy = row.querySelector<HTMLButtonElement>('[data-copy]')!;
        expect(copy.textContent!.trim()).toBe(en['create.copy']);
    });

    it('sums up the submitted capsule: items, size, expiry, password and burn', async () => {
        type('abc');
        add('code');
        type('de');
        field<HTMLSelectElement>('create-expiry').value = '3600';
        field<HTMLInputElement>('create-password').value = 'pass phrase';
        field<HTMLInputElement>('create-burn').checked = true;
        submit();
        await vi.waitFor(() => expect(document.querySelector('[data-summary]')).not.toBeNull(), {
            timeout: 30_000,
        });
        expect(chips()).toEqual([
            '2 items · 5 B',
            'Expires in 1 hour',
            'With password',
            'Deleted after first retrieval',
        ]);
    }, 30_000);

    it('sums up a capsule without password, expiry or burn, leaving out empty items', async () => {
        type('abc');
        add('plain');
        field<HTMLSelectElement>('create-expiry').value = '';
        submit();
        await sealedShareLink();
        expect(chips()).toEqual(['1 item · 3 B', en['expiry.none'], 'Without password']);
    });

    it('sums up in German', async () => {
        await loadPage('de');
        type('abc');
        submit();
        await sealedShareLink();
        expect(document.querySelector('main h1')!.textContent).toBe('Versiegelt.');
        expect(chips()).toEqual(['1 Element · 3 B', 'Läuft in 7 Tagen ab', 'Ohne Passwort']);
    });

    it.each([
        ['share link', 'share-link', 'create.copyLink'],
        ['delete link', 'delete-link', 'create.copy'],
    ] as const)('copies the %s and shows "Copied" for 2 s', async (_name, target, label) => {
        type('content');
        submit();
        await sealedShareLink();
        const url = el(target).textContent!;
        const button = document.querySelector<HTMLButtonElement>(`[data-copy="${target}"]`)!;
        const region = document.querySelector('main [role="status"][aria-live="polite"]')!;

        vi.useFakeTimers();
        button.click();
        // Lets the clipboard's promise settle without moving the clock.
        await vi.advanceTimersByTimeAsync(0);
        expect(writeText).toHaveBeenCalledWith(url);
        expect(button.classList.contains('cb-copied')).toBe(true);
        // The button's own content waits in a hidden element while it shows "Copied".
        const shown = button.querySelector(':scope > span:not([hidden])')!;
        expect(shown.textContent).toBe(en['shell.copied']);
        expect(button.querySelector('svg[data-icon="check"]')).not.toBeNull();
        expect(region.textContent).toBe(en['shell.copiedToClipboard']);

        vi.advanceTimersByTime(1999);
        expect(button.classList.contains('cb-copied')).toBe(true);
        vi.advanceTimersByTime(1);
        expect(button.classList.contains('cb-copied')).toBe(false);
        expect(button.textContent!.trim()).toBe(en[label]);
        expect(button.querySelector('svg[data-icon="copy"]')).not.toBeNull();
        expect(region.textContent).toBe('');
    });
});
