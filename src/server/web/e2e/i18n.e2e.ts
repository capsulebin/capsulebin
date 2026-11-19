import { readFileSync } from 'node:fs';
import type { Browser, BrowserContext, Page } from '@playwright/test';
import { create } from './create';
import { type Guard, expect, installGuard, test } from './guard';

/**
 * The interface languages in a real engine: every page under a language prefix,
 * the German interface without English left over, the language switch, internal
 * links that keep the prefix, and the way a prefix-less share link reaches the
 * page in the reader's language with the key still in the fragment.
 *
 * Playwright's browsers default to `en-US`; `language-init.js` would move a page
 * under `/de/` to `/en/` there, so the German flows run with `locale: 'de-DE'`.
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

        // A fresh browser, nothing stored: the server picks /de/ by Accept-Language.
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

test('an English browser moves a /de/ link to /en/ until German is chosen', async ({
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
    expect(urls).toContain(`${origin}/en/p/${id}${fragment}`);
    expect(new URL(view.url()).pathname).toBe(`/en/p/${id}`);

    // The stored choice outranks the browser language, and Accept-Language too.
    await view.evaluate(() => localStorage.setItem('capsulebin.language', 'de'));
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

test('the language switch moves the start page to German', async ({ page }) => {
    await page.goto('/en/');
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

    await expect(view.locator('[data-language-notice]')).toHaveText(
        catalogue('en')['shell.languageNextPage'] ?? '',
    );
    expect(await view.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');
    expect(await view.evaluate(() => 'stillTheSamePage' in window)).toBe(true);
    expect(view.url()).toBe(opened);
    await expect(panel).toContainText('Still here');
});
