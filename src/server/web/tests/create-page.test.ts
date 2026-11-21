/**
 * The create page's item tabs wired to the real HTML: adding items through the
 * menu, names, type changes that would lose content, removal, order, and what
 * ends up in the capsule.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
    derivePasswordKeyFor,
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

    /** Submits and waits for the done view; returns the share link. */
    async function create(): Promise<string> {
        submit();
        await vi.waitFor(() => expect(input('share-link')).not.toBeNull(), { timeout: 15_000 });
        return input<HTMLInputElement>('share-link').value;
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
                expect(document.querySelector('.cb-spinner')).toBeNull();
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
        await vi.waitFor(() => expect(input('share-link')).not.toBeNull(), { timeout: 15_000 });
        const share = input<HTMLInputElement>('share-link').value;
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

    async function done(): Promise<string> {
        await vi.waitFor(() => expect(field('share-link')).not.toBeNull(), { timeout: 15_000 });
        return field<HTMLInputElement>('share-link').value;
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
