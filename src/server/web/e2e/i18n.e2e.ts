import { readFileSync } from 'node:fs';
import type { Browser, BrowserContext, Page } from '@playwright/test';
import { create } from './create';
import { type Guard, expect, installGuard, test } from './guard';

/**
 * The interface languages in a real engine: the default language (`en` here)
 * without a path prefix and German under `/de/`, the German interface without
 * English left over, the language switch, internal links that stay in the page's
 * language, and the way a prefix-less share link reaches the page in the reader's
 * language with the key still in the fragment.
 *
 * Playwright's browsers default to `en-US`; `language-init.js` would move a page
 * under `/de/` to the prefix-less one there, so the German flows run with
 * `locale: 'de-DE'`.
 */

const catalogue = (language: string): Record<string, string> =>
    JSON.parse(
        readFileSync(new URL(`../src/i18n/${language}.json`, import.meta.url), 'utf8'),
    ) as Record<string, string>;

/**
 * English texts that must not appear on a German page: the literal parts of each
 * `en` value that differs from its `de` value, longer than 3 characters.
 *
 * Values with placeholders (`Expires in {count} minutes`) never appear verbatim,
 * so they are split at the placeholders and each literal part is checked on its
 * own; leaving them out would miss most error and status texts. A part that the
 * German value contains as well (`☰`, a unit) says nothing about the language.
 *
 * A part only counts as whole words (`Capsule` is in `CapsuleBin` on every page)
 * and case-insensitively, since `innerText` follows CSS `text-transform`.
 */
const ENGLISH_ONLY: RegExp[] = (() => {
    const en = catalogue('en');
    const de = catalogue('de');
    const parts = new Set<string>();
    for (const [key, value] of Object.entries(en)) {
        const german = de[key] ?? '';
        if (value === german) continue;
        for (const part of value.split(/\{\w+\}/)) {
            const literal = part.trim().replace(/\s+/g, ' ').toLowerCase();
            if (literal.length > 3 && !german.toLowerCase().includes(literal)) parts.add(literal);
        }
    }
    const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return [...parts].map(
        (part) => new RegExp(`(?<![\\p{L}\\p{N}])${escape(part)}(?![\\p{L}\\p{N}])`, 'iu'),
    );
})();

/** The visible text of the page has German in it and nothing from the English catalogue. */
async function expectGerman(page: Page, expected: string): Promise<void> {
    const body = page.locator('body');
    await expect(body).toContainText(expected, { timeout: 30_000 });
    const text = (await body.innerText()).replace(/\s+/g, ' ');
    expect(ENGLISH_ONLY.filter((english) => english.test(text)).map(String)).toEqual([]);
}

/** The `href` of every link to a local path, except the API, assets and scripts. */
async function internalLinks(page: Page): Promise<string[]> {
    const hrefs = await page
        .locator('a[href^="/"]')
        .evaluateAll((links) => links.map((link) => link.getAttribute('href') ?? ''));
    return hrefs.filter(
        (href) => !href.startsWith('/api/') && !href.startsWith('/assets/') && !/\.js$/.test(href),
    );
}

async function expectLinksUnder(page: Page, prefix: string): Promise<void> {
    const links = await internalLinks(page);
    expect(links.length).toBeGreaterThan(0);
    expect(links.filter((href) => !href.startsWith(prefix))).toEqual([]);
}

/** Pages in the default language link without any language prefix. */
async function expectLinksWithoutPrefix(page: Page): Promise<void> {
    const links = await internalLinks(page);
    expect(links.length).toBeGreaterThan(0);
    expect(links.filter((href) => /^\/(en|de)(\/|$)/.test(href))).toEqual([]);
}

/** A second browser with its own locale, under the same guard as the test's own. */
async function browserIn(
    browser: Browser,
    locale: string,
): Promise<{ context: BrowserContext; guard: Guard }> {
    const context = await browser.newContext({ locale });
    return { context, guard: await installGuard(context) };
}

/** The id and fragment of a share or management link. */
function parts(link: string): { id: string; fragment: string } {
    const url = new URL(link);
    return { id: url.pathname.split('/').at(-1) ?? '', fragment: url.hash };
}

/**
 * Every URL the main frame of `page` shows, including the ones `history.replaceState`
 * sets: the view page removes the fragment once the capsule is open, so the URL
 * it landed on is only visible on the way.
 */
function recordUrls(page: Page): string[] {
    const urls: string[] = [];
    page.on('framenavigated', (frame) => {
        if (frame === page.mainFrame()) urls.push(frame.url());
    });
    return urls;
}

test.describe('in a German browser', () => {
    test.use({ locale: 'de-DE' });

    test('create, open, delete and report stay German throughout', async ({ page, context }) => {
        const password = 'richtiges Kennwort';

        await page.goto('/de/');
        await expectGerman(page, 'Verschlüsselt teilen, ohne Konto');
        // The legal links in the footer: German labels from the web build, the server's targets.
        for (const [name, href] of [
            ['Impressum', '/de/imprint'],
            ['Datenschutz', '/de/privacy'],
            ['Nutzungsbedingungen', '/de/terms'],
        ] as const) {
            await expect(
                page.locator('footer').getByRole('link', { name, exact: true }),
            ).toHaveAttribute('href', href);
        }
        const { share, manage } = await create(
            page,
            { password, items: [{ type: 'plain', text: 'Geheimer Inhalt' }] },
            'de',
        );
        await expectGerman(page, 'Deine Kapsel ist fertig');

        const view = await context.newPage();
        await view.goto(share);
        await expect(view).toHaveURL(/\/de\/p\//);
        await expectGerman(view, 'Diese Kapsel ist mit einem Passwort geschützt');
        await view.fill('#pwd-input', 'falsches Kennwort');
        await view.click('button[type=submit]');
        await expect(view.locator('.cb-error')).toHaveText(
            'Falsches Passwort, oder die Kapsel ist nicht mehr verfügbar.',
            { timeout: 30_000 },
        );
        await expectGerman(view, 'Falsches Passwort');
        await view.fill('#pwd-input', password);
        await view.click('button[type=submit]');
        await expect(view.locator('.cb-item-panel')).toContainText('Geheimer Inhalt', {
            timeout: 30_000,
        });
        await expectGerman(view, 'Die Entschlüsselung findet lokal statt.');

        const managePage = await context.newPage();
        managePage.on('dialog', (dialog) => void dialog.accept());
        await managePage.goto(manage);
        await expect(managePage).toHaveURL(/\/de\/manage\//);
        await expectGerman(managePage, 'Ja, endgültig löschen');
        await managePage.click('[data-delete-btn]');
        await expectGerman(managePage, 'Die Kapsel wurde endgültig gelöscht.');

        await view.locator('footer').getByRole('link', { name: 'Inhalt melden' }).click();
        await expect(view).toHaveURL(/\/de\/report\?id=/);
        await expectGerman(view, 'Link oder ID der Kapsel');
        await view.selectOption('[data-report-reason]', 'spam');
        await view.fill('[data-report-description]', 'Prüfung des Meldeformulars.');
        await view.click('[data-report-submit]');
        await expectGerman(view, 'Meldung eingegangen');
    });

    test('every internal link keeps the /de/ prefix', async ({ page, context }) => {
        await page.goto('/de/');
        await expectLinksUnder(page, '/de/');
        const { share, manage } = await create(
            page,
            { items: [{ type: 'plain', text: 'Verweise' }] },
            'de',
        );
        await expectLinksUnder(page, '/de/');

        const view = await context.newPage();
        await view.goto(share);
        await expect(view.locator('.cb-item-panel')).toContainText('Verweise', {
            timeout: 30_000,
        });
        await expectLinksUnder(view, '/de/');
        expect(await internalLinks(view)).toContain(`/de/report?id=${parts(share).id}`);

        const managePage = await context.newPage();
        await managePage.goto(manage);
        await expect(managePage.locator('[data-delete-btn]')).toBeVisible({ timeout: 30_000 });
        await expectLinksUnder(managePage, '/de/');

        // The report page and the legal pages configured in playwright.config.ts.
        for (const path of ['/de/report', '/de/imprint', '/de/privacy', '/de/terms']) {
            await page.goto(path);
            await expect(page.locator('footer')).toBeVisible();
            await expectLinksUnder(page, '/de/');
        }
    });

    test('a prefix-less share link opens under /de/ with the key', async ({ page, browser }) => {
        const { share, manage } = await create(page, {
            items: [{ type: 'plain', text: 'Ohne Präfix geteilt' }],
        });
        const origin = new URL(page.url()).origin;
        const { id, fragment } = parts(share);
        expect(share).toBe(`${origin}/p/${id}${fragment}`);
        expect(fragment).toMatch(/^#v1\./);
        expect(manage).toMatch(new RegExp(`^${origin}/manage/${id}#v1\\.`));

        // A fresh browser, nothing stored: the server answers with the English page
        // and language-init.js moves it to /de/ by the browser language.
        const { context, guard } = await browserIn(browser, 'de-DE');
        const view = await context.newPage();
        const urls = recordUrls(view);
        await view.goto(share);
        await expect(view.locator('.cb-item-panel')).toContainText('Ohne Präfix geteilt', {
            timeout: 30_000,
        });
        expect(urls).toContain(`${origin}/de/p/${id}${fragment}`);
        expect(new URL(view.url()).pathname).toBe(`/de/p/${id}`);
        await context.close();
        expect(guard.problems).toEqual([]);
    });
});

test('every internal link of an English page has no language prefix', async ({ page, context }) => {
    await page.goto('/');
    await expectLinksWithoutPrefix(page);
    const { share, manage } = await create(page, { items: [{ type: 'plain', text: 'Links' }] });
    await expectLinksWithoutPrefix(page);

    const view = await context.newPage();
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toContainText('Links', { timeout: 30_000 });
    await expectLinksWithoutPrefix(view);
    expect(await internalLinks(view)).toContain(`/report?id=${parts(share).id}`);

    const managePage = await context.newPage();
    await managePage.goto(manage);
    await expect(managePage.locator('[data-delete-btn]')).toBeVisible({ timeout: 30_000 });
    await expectLinksWithoutPrefix(managePage);

    for (const path of ['/report', '/imprint', '/privacy', '/terms']) {
        await page.goto(path);
        await expect(page.locator('footer')).toBeVisible();
        await expectLinksWithoutPrefix(page);
    }
});

test('an English browser moves a /de/ link to the prefix-less one until German is chosen', async ({
    page,
    browser,
}) => {
    const { share } = await create(page, { items: [{ type: 'plain', text: 'Moved along' }] });
    const origin = new URL(page.url()).origin;
    const { id, fragment } = parts(share);

    const { context, guard } = await browserIn(browser, 'en-US');
    const view = await context.newPage();
    const urls = recordUrls(view);
    await view.goto(`${origin}/de/p/${id}${fragment}`);
    await expect(view.locator('.cb-item-panel')).toContainText('Moved along', { timeout: 30_000 });
    expect(urls).toContain(`${origin}/p/${id}${fragment}`);
    expect(new URL(view.url()).pathname).toBe(`/p/${id}`);

    // The stored choice outranks the browser language: the /de/ link now stays ...
    await view.evaluate(() => localStorage.setItem('capsulebin.language', 'de'));
    const stays = await context.newPage();
    const staysUrls = recordUrls(stays);
    await stays.goto(`${origin}/de/p/${id}${fragment}`);
    await expect(stays.locator('.cb-item-panel')).toContainText('Moved along', {
        timeout: 30_000,
    });
    expect(staysUrls).not.toContain(`${origin}/p/${id}${fragment}`);
    expect(new URL(stays.url()).pathname).toBe(`/de/p/${id}`);

    // ... and the prefix-less share link moves to /de/.
    const again = await context.newPage();
    const againUrls = recordUrls(again);
    await again.goto(share);
    await expect(again.locator('.cb-item-panel')).toContainText('Moved along', {
        timeout: 30_000,
    });
    expect(againUrls).toContain(`${origin}/de/p/${id}${fragment}`);
    expect(new URL(again.url()).pathname).toBe(`/de/p/${id}`);
    await context.close();
    expect(guard.problems).toEqual([]);
});

test('on a filled create form the language switch only stores the choice', async ({ page }) => {
    await page.goto('/');
    const editor = page.locator('[data-item-text]');
    await editor.fill('A draft worth keeping');
    await page.evaluate(() => Object.assign(window, { stillTheSamePage: true }));

    await page.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();
    await expect(page.locator('[data-language-notice]')).toHaveText(
        catalogue('de')['shell.languageNextPage'] ?? '',
    );
    expect(await page.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');
    expect(await page.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(page.url()).pathname).toBe('/');
    await expect(editor).toHaveValue('A draft worth keeping');
});

// AC24: a field other than the items is a draft too.
test('with only a title or an expiry set the language switch only stores the choice', async ({
    page,
}) => {
    const german = page.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' });
    const notice = page.locator('[data-language-notice]');

    await page.goto('/');
    await page.locator('#create-title').fill('Only a title');
    await page.evaluate(() => Object.assign(window, { stillTheSamePage: true }));
    await german.click();
    await expect(notice).toHaveText(catalogue('de')['shell.languageNextPage'] ?? '');
    expect(await page.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(page.url()).pathname).toBe('/');
    await expect(page.locator('#create-title')).toHaveValue('Only a title');

    await page.evaluate(() => localStorage.clear());
    await page.goto('/');
    await page.locator('#create-expiry').selectOption('3600');
    await page.evaluate(() => Object.assign(window, { stillTheSamePage: true }));
    await german.click();
    await expect(notice).toHaveText(catalogue('de')['shell.languageNextPage'] ?? '');
    expect(await page.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(page.url()).pathname).toBe('/');
    await expect(page.locator('#create-expiry')).toHaveValue('3600');
});

// On an empty create form the switch navigates.
test('the language switch moves the start page to German and back', async ({ page }) => {
    await page.goto('/');
    const switcher = page.locator('[data-language-switch]');
    await expect(switcher).toBeVisible();
    await expect(switcher.getByRole('button', { name: 'English' })).toHaveAttribute(
        'aria-current',
        'true',
    );

    await switcher.getByRole('button', { name: 'Deutsch' }).click();
    await expect(page).toHaveURL(/\/de\/$/);
    await expect(page.locator('html')).toHaveAttribute('lang', 'de');
    expect(await page.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');
    await expect(switcher.getByRole('button', { name: 'Deutsch' })).toHaveAttribute(
        'aria-current',
        'true',
    );
    await expect(switcher.getByRole('button', { name: 'English' })).not.toHaveAttribute(
        'aria-current',
    );

    // The default language has no prefix.
    await switcher.getByRole('button', { name: 'English' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');
    expect(new URL(page.url()).pathname).toBe('/');
    expect(await page.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('en');
});

test('on an opened capsule the language switch only stores the choice', async ({
    page,
    context,
}) => {
    const { share } = await create(page, { items: [{ type: 'plain', text: 'Still here' }] });
    const view = await context.newPage();
    await view.goto(share);
    const panel = view.locator('.cb-item-panel');
    await expect(panel).toContainText('Still here', { timeout: 30_000 });
    const opened = view.url();

    // Reloading would lose the key: a marker on the window survives only without navigation.
    await view.evaluate(() => Object.assign(window, { stillTheSamePage: true }));
    await view.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();

    // AC27: in the chosen language, on the English page.
    await expect(view.locator('html')).toHaveAttribute('lang', 'en');
    const notice = view.locator('[data-language-notice]');
    await expect(notice).toHaveText(catalogue('de')['shell.languageNextPage'] ?? '');
    await expect(notice).toHaveAttribute('lang', 'de');
    expect(await view.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');
    expect(await view.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(view.url()).toBe(opened);
    await expect(panel).toContainText('Still here');
});

test('while a burn capsule is retrieved the language switch only stores the choice', async ({
    page,
    context,
}) => {
    const { share } = await create(page, {
        items: [{ type: 'plain', text: 'Shown once' }],
        burn: true,
    });
    const view = await context.newPage();

    // Hold the retrieve request until the language has been switched.
    let release = () => {};
    const held = new Promise<void>((resolve) => (release = resolve));
    let reached = () => {};
    const requested = new Promise<void>((resolve) => (reached = resolve));
    await view.route('**/api/v1/pastes/*/retrieve', async (route) => {
        reached();
        await held;
        await route.continue();
    });

    await view.goto(share);
    await view.evaluate(() => Object.assign(window, { stillTheSamePage: true }));
    await view.getByRole('button', { name: 'Yes, open and burn' }).click();
    await requested;

    await view.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();
    await expect(view.locator('[data-language-notice]')).toHaveText(
        catalogue('de')['shell.languageNextPage'] ?? '',
    );
    release();

    await expect(view.locator('.cb-item-panel')).toContainText('Shown once', { timeout: 30_000 });
    expect(await view.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(await view.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');
});

// The other order of the same click: confirmed first, then the switch at once.
test('right after the burn confirmation the language switch only stores the choice', async ({
    page,
    context,
}) => {
    const { share } = await create(page, {
        items: [{ type: 'plain', text: 'Confirmed first' }],
        burn: true,
    });
    const view = await context.newPage();
    await view.goto(share);
    const opened = new URL(view.url()).pathname;
    await view.evaluate(() => Object.assign(window, { stillTheSamePage: true }));

    await view.getByRole('button', { name: 'Yes, open and burn' }).click();
    await view.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();

    await expect(view.locator('[data-language-notice]')).toHaveText(
        catalogue('de')['shell.languageNextPage'] ?? '',
    );
    await expect(view.locator('.cb-item-panel')).toContainText('Confirmed first', {
        timeout: 30_000,
    });
    expect(await view.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(view.url()).pathname).toBe(opened);
    await expect(view.locator('html')).toHaveAttribute('lang', 'en');
});

// The old document stays on screen until the German one arrives. Confirming the
// burn there would retrieve the capsule behind a page that is about to go, and
// the German page would find it gone.
test('once the language switch is loading the page a burn capsule cannot be opened', async ({
    page,
    context,
}) => {
    const { share } = await create(page, {
        items: [{ type: 'plain', text: 'Not burned unseen' }],
        burn: true,
    });
    const view = await context.newPage();
    const retrieves: string[] = [];
    view.on('request', (request) => {
        if (request.url().endsWith('/retrieve')) retrieves.push(request.url());
    });

    // The German page takes its time, and Playwright cannot look into the old
    // document while it waits for the new one: the old document itself tries the
    // confirmation, and notes for the next one what it saw.
    await view.route('**/de/p/*', async (route) => {
        await new Promise((resolve) => setTimeout(resolve, 2000));
        await route.continue();
    });

    await view.goto(share);
    const confirm = view.getByRole('button', { name: 'Yes, open and burn' });
    await expect(confirm).toBeVisible({ timeout: 30_000 });
    await confirm.evaluate((button: HTMLButtonElement) => {
        document.querySelector<HTMLButtonElement>('[data-language-switch] [lang="de"]')?.click();
        setTimeout(() => {
            sessionStorage.setItem('confirmWasDisabled', String(button.disabled));
            // Even a click that still gets through starts nothing.
            button.disabled = false;
            button.click();
        }, 300);
    });

    // The capsule is still there, on the German page.
    await expect(view).toHaveURL(/\/de\/p\/.*#v1\./);
    await expect(view.locator('html')).toHaveAttribute('lang', 'de');
    expect(await view.evaluate(() => sessionStorage.getItem('confirmWasDisabled'))).toBe('true');
    expect(retrieves).toEqual([]);
    await view.getByRole('button', { name: catalogue('de')['view.burnConfirm'] ?? '' }).click();
    await expect(view.locator('.cb-item-panel')).toContainText('Not burned unseen', {
        timeout: 30_000,
    });
    expect(retrieves).toHaveLength(1);
});

// A started report is a draft like a filled create form; nothing of it is stored.
test('on a started report the language switch only stores the choice', async ({ page }) => {
    await page.goto('/report');
    const description = page.locator('[data-report-description]');
    await description.fill('A report worth keeping');
    await page.evaluate(() => Object.assign(window, { stillTheSamePage: true }));

    await page.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();
    await expect(page.locator('[data-language-notice]')).toHaveText(
        catalogue('de')['shell.languageNextPage'] ?? '',
    );
    expect(await page.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(page.url()).pathname).toBe('/report');
    await expect(description).toHaveValue('A report worth keeping');
    expect(
        await page.evaluate(() => [Object.keys(localStorage), Object.keys(sessionStorage)]),
    ).toEqual([['capsulebin.language'], []]);
});

test('the language switch leaves no history entry with the key', async ({ page, context }) => {
    const password = 'correct password';
    const { share } = await create(page, {
        password,
        items: [{ type: 'plain', text: 'Behind a password' }],
    });
    const view = await context.newPage();
    await view.goto(share);
    await expect(view.locator('#pwd-input')).toBeVisible({ timeout: 30_000 });

    // The fragment is still there, so the switch loads the page under /de/.
    await view.locator('[data-language-switch]').getByRole('button', { name: 'Deutsch' }).click();
    await expect(view).toHaveURL(/\/de\/p\/.*#v1\./);
    await expect(view.locator('#pwd-input')).toBeVisible({ timeout: 30_000 });
    await view.fill('#pwd-input', password);
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-item-panel')).toContainText('Behind a password', {
        timeout: 30_000,
    });
    expect(view.url()).not.toContain('#v1.');

    await view.goBack();
    expect(view.url()).not.toContain('#v1.');
});

// AC13 on the management page: while the delete key is still in the fragment the
// switch loads the page in the other language and the key goes along, so the
// capsule can still be deleted there; once it is deleted and the fragment is
// gone, the switch only stores the choice and says so in the chosen language.
test('on the management page the language switch keeps the key, and only stores after the delete', async ({
    page,
    context,
}) => {
    const { share, manage } = await create(page, {
        items: [{ type: 'plain', text: 'Managed in two languages' }],
    });
    const { id, fragment } = parts(manage);
    expect(fragment).toMatch(/^#v1\./);

    const managePage = await context.newPage();
    managePage.on('dialog', (dialog) => void dialog.accept());
    await managePage.goto(manage);
    await expect(managePage.locator('[data-delete-btn]')).toBeVisible({ timeout: 30_000 });
    const switcher = managePage.locator('[data-language-switch]');

    await switcher.getByRole('button', { name: 'Deutsch' }).click();
    await expect(managePage.locator('html')).toHaveAttribute('lang', 'de');
    expect(new URL(managePage.url()).pathname).toBe(`/de/manage/${id}`);
    expect(new URL(managePage.url()).hash).toBe(fragment);
    expect(await managePage.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');

    // The key arrived with the page: the delete works from the German page.
    await expect(managePage.locator('[data-delete-btn]')).toBeVisible({ timeout: 30_000 });
    await managePage.click('[data-delete-btn]');
    await expect(managePage.locator('body')).toContainText(
        catalogue('de')['manage.deletedLede'] ?? '',
        { timeout: 30_000 },
    );
    expect(new URL(managePage.url()).hash).toBe('');

    await managePage.evaluate(() => Object.assign(window, { stillTheSamePage: true }));
    await switcher.getByRole('button', { name: 'English' }).click();
    const notice = managePage.locator('[data-language-notice]');
    await expect(notice).toHaveText(catalogue('en')['shell.languageNextPage'] ?? '');
    await expect(notice).toHaveAttribute('lang', 'en');
    expect(await managePage.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('en');
    expect(await managePage.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(new URL(managePage.url()).pathname).toBe(`/de/manage/${id}`);
    await expect(managePage.locator('html')).toHaveAttribute('lang', 'de');

    // The capsule is really gone.
    const view = await context.newPage();
    await view.goto(share);
    await expect(view.locator('h1')).toHaveText('Not available', { timeout: 30_000 });
});
