import { fileURLToPath } from 'node:url';
import type { APIRequestContext, Browser, Page } from '@playwright/test';
import { prefixOf } from './create';
import { expect, installGuard, test as base } from './guard';
import { withInstance } from './instance';

/**
 * The instance identity (`[branding]`): name, logo, dark logo, favicon and the
 * attribution link, on the pages the server fills from the shells of the web
 * build, in a real engine under the CSP. The guard fails a test on any CSP or
 * Trusted Types violation and any console error.
 *
 * The main instance of playwright.config.ts has no `[branding]` and shows the
 * built-in identity. More instances are started per worker when a test needs
 * them: one with every image, a name and taglines that need escaping, one with
 * only a logo, and one with only an empty tagline.
 */

const fixture = (name: string) => fileURLToPath(new URL(`fixtures/${name}`, import.meta.url));

const NAME = 'Acme & Co';
const SOURCE_URL = 'https://example.org/acme/capsulebin?fork=1&ref=e2e';
const DEFAULT_SOURCE_URL = 'https://github.com/capsulebin/capsulebin';
const TAGLINE = { en: 'Seal <it> & "send" it', de: 'Versiegeln & <teilen>' } as const;

const test = base.extend<object, { branded: string; logoOnly: string; noTagline: string }>({
    branded: [
        async ({}, use) =>
            withInstance(
                {
                    CAPSULEBIN__BRANDING__NAME: NAME,
                    CAPSULEBIN__BRANDING__LOGO: fixture('logo.png'),
                    CAPSULEBIN__BRANDING__LOGO_DARK: fixture('logo-dark.webp'),
                    CAPSULEBIN__BRANDING__FAVICON: fixture('favicon.png'),
                    CAPSULEBIN__BRANDING__SOURCE_URL: SOURCE_URL,
                    CAPSULEBIN__BRANDING__TAGLINE: TAGLINE.en,
                    CAPSULEBIN__BRANDING__DE__TAGLINE: TAGLINE.de,
                    CAPSULEBIN__LEGAL__OPERATOR__TEXT: 'Example Operator GmbH',
                },
                use,
            ),
        { scope: 'worker' },
    ],
    logoOnly: [
        async ({}, use) => withInstance({ CAPSULEBIN__BRANDING__LOGO: fixture('logo.png') }, use),
        { scope: 'worker' },
    ],
    noTagline: [
        async ({}, use) => withInstance({ CAPSULEBIN__BRANDING__TAGLINE: '' }, use),
        { scope: 'worker' },
    ],
});

const SOME_ID = '2pS3xQ7Rk9mNpLz4AbCdEA';

/**
 * One page of each shell, with its title before the name (none on the create
 * page) and the text next to the logo (`.cb-claim`; the view page has none, the
 * create page shows the slogan, see {@link SLOGAN}).
 */
const PAGES = {
    en: [
        ['/', '', ''],
        [`/p/${SOME_ID}`, 'Open capsule', null],
        [`/manage/${SOME_ID}`, 'Manage capsule', 'Manage'],
        ['/report', 'Report content', 'Report content'],
        ['/imprint', 'Imprint', 'Imprint'],
    ],
    de: [
        ['/', '', ''],
        [`/p/${SOME_ID}`, 'Kapsel öffnen', null],
        [`/manage/${SOME_ID}`, 'Kapsel verwalten', 'Verwalten'],
        ['/report', 'Inhalt melden', 'Inhalt melden'],
        ['/imprint', 'Impressum', 'Impressum'],
    ],
} as const;

/** The built-in slogan of the create page (`create.claim`). */
const SLOGAN = { en: 'Paste it. Seal it. Share it.', de: 'Einfügen. Versiegeln. Teilen.' } as const;

const ATTRIBUTION = { en: 'Powered by CapsuleBin', de: 'Betrieben mit CapsuleBin' } as const;

const LANGUAGES = [
    ['en', 'en-US'],
    ['de', 'de-DE'],
] as const;

const title = (page: string, name: string) => (page ? `${page} · ${name}` : name);

/** The link to the source code, relative to whatever it is queried in. */
const projectLink = (page: Page) => page.getByRole('link', { name: 'CapsuleBin', exact: true });

/** The footer's one link to the source code. */
const attributionLink = (page: Page) => page.locator('footer').locator(projectLink(page));

/**
 * The footer has exactly one attribution: the item reads "Powered by CapsuleBin"
 * with only the project name as a link to `href`, opening in a new tab (AC10).
 */
async function expectAttribution(page: Page, language: 'en' | 'de', href: string): Promise<void> {
    const footer = page.locator('footer');
    const link = attributionLink(page);
    await expect(link).toHaveCount(1);
    await expect(footer.locator('a').filter({ hasText: 'CapsuleBin' })).toHaveCount(1);
    await expect(link).toHaveText('CapsuleBin');
    await expect(link).toHaveAttribute('href', href);
    await expect(link).toHaveAttribute('target', '_blank');
    await expect(link).toHaveAttribute('rel', 'noopener noreferrer');

    const item = footer.locator('.cb-foot-links > span').filter({ has: projectLink(page) });
    await expect(item).toHaveCount(1);
    await expect(item).toHaveText(ATTRIBUTION[language]);
    await expect(item.locator('a')).toHaveCount(1);
}

/** The text next to the logo: `claim`, or no such element for `null` (AC15). */
async function expectClaim(page: Page, claim: string | null): Promise<void> {
    const element = page.locator('header .cb-claim');
    if (claim === null) {
        await expect(element).toHaveCount(0);
    } else {
        await expect(element).toHaveCount(1);
        await expect(element).toHaveText(claim);
    }
}

/** The `href` of the page's icon link answers 200 with `contentType` (AC9). */
async function expectFavicon(
    page: Page,
    request: APIRequestContext,
    contentType: string,
): Promise<string> {
    const icon = page.locator('link[rel="icon"]');
    await expect(icon).toHaveCount(1);
    const href = (await icon.getAttribute('href')) ?? '';
    const answer = await request.get(new URL(href, page.url()).href);
    expect(answer.status(), href).toBe(200);
    expect(answer.headers()['content-type'], href).toBe(contentType);
    return href;
}

/** Waits until `image` has loaded with real pixels. */
async function expectLoaded(page: Page, selector: string): Promise<void> {
    await expect
        .poll(() =>
            page
                .locator(selector)
                .evaluate((image: HTMLImageElement) => (image.complete ? image.naturalWidth : 0)),
        )
        .toBeGreaterThan(0);
}

for (const [language, locale] of LANGUAGES) {
    const prefix = prefixOf(language);

    test.describe(`without [branding], in ${language}`, () => {
        test.use({ locale });

        for (const [path, pageTitle, claim] of PAGES[language]) {
            test(`${prefix}${path} shows the built-in identity`, async ({ page, request }) => {
                await page.goto(`${prefix}${path}`);

                await expect(page).toHaveTitle(title(pageTitle, 'CapsuleBin'));
                const logo = page.locator('header .cb-logo');
                await expect(logo.locator('svg')).toHaveCount(1);
                await expect(logo.locator('svg')).toBeVisible();
                await expect(logo.locator('img')).toHaveCount(0);
                await expect(logo.locator('.cb-logo-name')).toHaveText('CapsuleBin');
                await expectClaim(page, path === '/' ? SLOGAN[language] : claim);

                expect(await expectFavicon(page, request, 'image/svg+xml')).toBe('/favicon.svg');
                await expectAttribution(page, language, DEFAULT_SOURCE_URL);
            });
        }
    });

    test(`a branded instance shows its identity in ${language}`, async ({
        branded,
        browser,
        request,
    }) => {
        const context = await browser.newContext({ locale, baseURL: branded });
        const guard = await installGuard(context);
        const page = await context.newPage();

        for (const [path, pageTitle, claim] of PAGES[language]) {
            await page.goto(`${prefix}${path}`);
            await expect(page.locator('html')).toHaveAttribute('lang', language);

            await expect(page, path).toHaveTitle(title(pageTitle, NAME));
            const logo = page.locator('header .cb-logo');
            await expect(logo.locator('svg')).toHaveCount(0);
            await expect(logo.locator('.cb-logo-name')).toHaveText(NAME);
            await expectClaim(page, path === '/' ? TAGLINE[language] : claim);
            const light = logo.locator('img.cb-logo-img-light');
            const dark = logo.locator('img.cb-logo-img-dark');
            await expect(logo.locator('img')).toHaveCount(2);
            await expect(light).toHaveAttribute('src', /^\/branding\/[0-9a-f]{16}\.png$/);
            await expect(dark).toHaveAttribute('src', /^\/branding\/[0-9a-f]{16}\.webp$/);
            await expect(light).toHaveAttribute('alt', '');
            await expect(dark).toHaveAttribute('alt', '');
            await expectLoaded(page, 'header img.cb-logo-img-light');
            await expectLoaded(page, 'header img.cb-logo-img-dark');

            expect(await expectFavicon(page, request, 'image/png')).toMatch(
                /^\/branding\/[0-9a-f]{16}\.png$/,
            );
            await expectAttribution(page, language, SOURCE_URL);
        }

        await context.close();
        expect(guard.problems).toEqual([]);
    });
}

test('an empty tagline removes the slogan of the create page', async ({ noTagline, browser }) => {
    for (const [language, locale] of LANGUAGES) {
        const context = await browser.newContext({ locale, baseURL: noTagline });
        const guard = await installGuard(context);
        const page = await context.newPage();
        await page.goto(`${prefixOf(language)}/`);
        await expect(page.locator('html')).toHaveAttribute('lang', language);
        await expect(page.locator('header .cb-logo-name')).toHaveText('CapsuleBin');
        await expectClaim(page, null);
        await context.close();
        expect(guard.problems).toEqual([]);
    }
});

test('the attribution opens the source in a new page and leaves the current one', async ({
    context,
    page,
}) => {
    // Never reach the real source host: the new page gets a local answer.
    await context.route(/^https:\/\/github\.com\//, (route) =>
        route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>source</title>' }),
    );
    await page.goto('/report');
    const here = page.url();

    const [opened] = await Promise.all([
        context.waitForEvent('page'),
        attributionLink(page).click(),
    ]);
    await expect(opened).toHaveURL(DEFAULT_SOURCE_URL);
    await expect(opened).toHaveTitle('source');
    expect(await opened.evaluate(() => window.opener)).toBeNull();
    expect(page.url()).toBe(here);
    await expect(attributionLink(page)).toBeVisible();
});

/**
 * Opens the create page of `origin` with the system preference `colorScheme`,
 * and returns the page with its theme toggle under the guard.
 */
async function openWithScheme(browser: Browser, origin: string, colorScheme: 'light' | 'dark') {
    const context = await browser.newContext({ locale: 'en-US', baseURL: origin, colorScheme });
    const guard = await installGuard(context);
    const page = await context.newPage();
    await page.goto('/');
    const toggle = page.locator('[data-theme-toggle]');
    await expect(toggle).toHaveAttribute('data-theme', 'auto');
    return { context, guard, page, toggle };
}

/** Exactly the logo of `theme` is visible in the header. */
async function expectLogo(page: Page, theme: 'light' | 'dark'): Promise<void> {
    const other = theme === 'light' ? 'dark' : 'light';
    await expect(page.locator(`header .cb-logo-img-${theme}`)).toBeVisible();
    await expect(page.locator(`header .cb-logo-img-${other}`)).toBeHidden();
    await expect(page.locator('header .cb-logo img:visible')).toHaveCount(1);
}

for (const scheme of ['light', 'dark'] as const) {
    test(`with a ${scheme} system preference the logo follows the theme toggle both ways`, async ({
        branded,
        browser,
    }) => {
        const { context, guard, page, toggle } = await openWithScheme(browser, branded, scheme);

        // No manual choice: the system preference decides.
        await expectLogo(page, scheme);

        // The toggle goes auto → light → dark → auto: light to dark, then back to the system.
        await toggle.click();
        await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
        await expectLogo(page, 'light');
        await toggle.click();
        await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
        await expectLogo(page, 'dark');
        await toggle.click();
        await expect(page.locator('html')).not.toHaveAttribute('data-theme');
        await expectLogo(page, scheme);

        // A stored dark choice holds on the next page (theme-init.js), and from there the
        // toggle leads back to light through the system preference.
        await toggle.click();
        await toggle.click();
        await page.reload();
        await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
        await expectLogo(page, 'dark');
        await toggle.click();
        await expect(page.locator('html')).not.toHaveAttribute('data-theme');
        await expectLogo(page, scheme);
        await toggle.click();
        await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
        await expectLogo(page, 'light');

        await context.close();
        expect(guard.problems).toEqual([]);
    });

    test(`without a dark logo the one logo shows in every theme, from a ${scheme} preference`, async ({
        logoOnly,
        browser,
    }) => {
        const { context, guard, page, toggle } = await openWithScheme(browser, logoOnly, scheme);
        const logo = page.locator('header .cb-logo img');
        await expect(logo).toHaveCount(1);
        await expectLoaded(page, 'header .cb-logo img');
        await expect(page.locator('header .cb-logo .cb-logo-name')).toHaveText('CapsuleBin');

        for (const theme of [null, 'light', 'dark']) {
            if (theme) await toggle.click();
            const root = page.locator('html');
            await (theme
                ? expect(root).toHaveAttribute('data-theme', theme)
                : expect(root).not.toHaveAttribute('data-theme'));
            await expect(logo).toBeVisible();
        }

        await context.close();
        expect(guard.problems).toEqual([]);
    });
}
