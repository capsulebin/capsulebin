import { prefixOf } from './create';
import { expect, test } from './guard';

/**
 * The operator's legal pages (`[legal]`, set in playwright.config.ts): linked
 * from the footer of every page and served by the server into `legal.html`,
 * under the same CSP as everything else. The guard fails the test on any
 * violation or console error.
 */

/** The footer labels of each language, from its catalogue; also the pages' titles. */
const LINKS = {
    en: [
        ['Imprint', '/imprint'],
        ['Privacy', '/privacy'],
        ['Terms', '/terms'],
    ],
    de: [
        ['Impressum', '/imprint'],
        ['Datenschutz', '/privacy'],
        ['Nutzungsbedingungen', '/terms'],
    ],
} as const;

for (const [language, locale] of [
    ['en', 'en-US'],
    ['de', 'de-DE'],
] as const) {
    const prefix = prefixOf(language);

    test.describe(`in ${language}`, () => {
        // A browser in that language, so language-init.js stays on the page.
        test.use({ locale });

        for (const path of [
            '/',
            '/p/2pS3xQ7Rk9mNpLz4AbCdEA',
            '/manage/2pS3xQ7Rk9mNpLz4AbCdEA',
            '/report',
            '/imprint',
        ]) {
            test(`the footer of ${prefix}${path} links to the legal pages`, async ({ page }) => {
                await page.goto(`${prefix}${path}`);

                const footer = page.locator('footer');
                for (const [name, href] of LINKS[language]) {
                    await expect(footer.getByRole('link', { name, exact: true })).toHaveAttribute(
                        'href',
                        `${prefix}${href}`,
                    );
                }
            });
        }

        for (const [title, path] of LINKS[language]) {
            test(`${prefix}${path} is titled with its footer label`, async ({ page }) => {
                await page.goto(`${prefix}${path}`);

                await expect(page).toHaveTitle(`${title} · CapsuleBin`);
                await expect(page.locator('header .cb-claim')).toHaveText(title);
                const heading = page.locator('.cb-legal').getByRole('heading', { level: 1 });
                await expect(heading).toHaveText(title);
                // The heading is in the page language, whatever language the text below has.
                await expect(
                    heading.locator('xpath=ancestor-or-self::*[@lang][1]'),
                ).toHaveAttribute('lang', language);
            });
        }
    });
}

test('the imprint names the operator, then the hosting provider', async ({ page }) => {
    await page.goto('/imprint');

    const article = page.locator('.cb-legal article');
    await expect(article).not.toHaveAttribute('lang');
    await expect(article).toContainText('Example Operator GmbH');
    await expect(article.getByRole('heading', { level: 2 })).toHaveText('Hosting');
    await expect(article).toContainText('Example Hosting AG');
});

test('the built-in privacy notice and terms point to each other and to the imprint', async ({
    page,
}) => {
    await page.goto('/terms');

    await page.locator('.cb-legal article').getByRole('link', { name: 'privacy notice' }).click();
    await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/privacy$/);
    const privacy = page.locator('.cb-legal article');
    await expect(privacy).not.toHaveAttribute('lang');
    await expect(privacy.getByRole('heading', { name: 'Controller' })).toBeVisible();
    await expect(privacy).toContainText('Art. 28 GDPR');

    await privacy.getByRole('link', { name: 'imprint' }).click();
    await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/imprint$/);
});

test.describe('in German', () => {
    test.use({ locale: 'de-DE' });

    test('the imprint marks the English operator text, not its heading', async ({ page }) => {
        await page.goto('/de/imprint');

        await expect(page.locator('html')).toHaveAttribute('lang', 'de');
        await expect(page.locator('.cb-legal h1')).toHaveText('Impressum');
        // Only [legal.*] is configured: the whole text falls back and says so.
        const article = page.locator('.cb-legal article');
        await expect(article).toHaveAttribute('lang', 'en');
        await expect(article).toContainText('Example Operator GmbH');
        await expect(article).toContainText('Example Hosting AG');
    });

    test('the built-in privacy notice and terms are German and point to the German imprint', async ({
        page,
    }) => {
        await page.goto('/de/terms');
        const terms = page.locator('.cb-legal article');
        await expect(terms).not.toHaveAttribute('lang');
        await expect(terms.getByRole('heading', { name: 'Verfügbarkeit' })).toBeVisible();

        await terms.getByRole('link', { name: 'Datenschutzerklärung' }).click();
        await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/de\/privacy$/);
        await expect(page.locator('.cb-legal h1')).toHaveText('Datenschutz');
        const privacy = page.locator('.cb-legal article');
        await expect(privacy).not.toHaveAttribute('lang');
        await expect(privacy.locator('[lang]')).toHaveCount(0);
        await expect(privacy.getByRole('heading', { name: 'Verantwortlicher' })).toBeVisible();
        await expect(privacy).toContainText('Art. 28 DSGVO');

        await privacy.getByRole('link', { name: 'Impressum' }).click();
        await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/de\/imprint$/);
    });

    // Each text's own cross-reference (not the footer's link of the same name).
    for (const [path, heading, german, link, target] of [
        ['/de/privacy', 'Datenschutz', 'DSGVO', 'Impressum', '/de/imprint'],
        [
            '/de/terms',
            'Nutzungsbedingungen',
            'Verfügbarkeit',
            'Datenschutzerklärung',
            '/de/privacy',
        ],
    ] as const) {
        test(`${path} opened directly is German and links to ${target}`, async ({ page }) => {
            await page.goto(path);

            await expect(page.locator('html')).toHaveAttribute('lang', 'de');
            await expect(page.locator('.cb-legal h1')).toHaveText(heading);
            const article = page.locator('.cb-legal article');
            await expect(article).not.toHaveAttribute('lang');
            await expect(article).toContainText(german);
            await expect(article).not.toContainText(/GDPR|Availability|Your rights/);
            await expect(article.getByRole('link', { name: link, exact: true })).toHaveAttribute(
                'href',
                target,
            );
            await expect(page.locator('a[href="/imprint"]')).toHaveCount(0);
        });
    }
});

// AC13: the built-in texts' pages themselves (not only the other pages) lead to
// the imprint of their own language; the German terms do so through the footer.
for (const [prefix, locale, imprint] of [
    ['', 'en-US', 'Imprint'],
    ['/de', 'de-DE', 'Impressum'],
] as const) {
    test.describe(`the built-in texts under ${prefix || '/'}`, () => {
        test.use({ locale });

        for (const path of ['/privacy', '/terms']) {
            test(`${prefix}${path} links to ${prefix}/imprint and nowhere else's`, async ({
                page,
            }) => {
                await page.goto(`${prefix}${path}`);

                const footer = page.locator('footer');
                await expect(
                    footer.getByRole('link', { name: imprint, exact: true }),
                ).toHaveAttribute('href', `${prefix}/imprint`);
                const other = prefix === '' ? '/de/imprint' : '/imprint';
                await expect(page.locator(`a[href="${other}"]`)).toHaveCount(0);

                await footer.getByRole('link', { name: imprint, exact: true }).click();
                await expect(page).toHaveURL(new RegExp(`^[a-z]+://[^/]+${prefix}/imprint$`));
                await expect(page.locator('.cb-legal h1')).toHaveText(imprint);
            });
        }
    });
}
