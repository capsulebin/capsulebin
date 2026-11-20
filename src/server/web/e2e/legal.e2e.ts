import { prefixOf } from './create';
import { expect, test } from './guard';

/**
 * The operator's legal pages (`[legal]`, set in playwright.config.ts): linked
 * from the footer of every page and served by the server into `legal.html`,
 * under the same CSP as everything else. The guard fails the test on any
 * violation or console error.
 */

/** The footer labels of each language, from its catalogue. */
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
    });
}

test('the imprint names the operator, then the hosting provider', async ({ page }) => {
    await page.goto('/imprint');

    await expect(page).toHaveTitle('Imprint · CapsuleBin');
    const article = page.locator('article.cb-legal');
    await expect(article.getByRole('heading', { level: 1 })).toHaveText('Imprint');
    await expect(article).toContainText('Example Operator GmbH');
    await expect(article.getByRole('heading', { level: 2 })).toHaveText('Hosting');
    await expect(article).toContainText('Example Hosting AG');
});

test('the built-in privacy notice and terms point to each other and to the imprint', async ({
    page,
}) => {
    await page.goto('/terms');
    await expect(page.locator('article.cb-legal h1')).toHaveText('Terms of use');

    await page.locator('article.cb-legal').getByRole('link', { name: 'privacy notice' }).click();
    await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/privacy$/);
    const privacy = page.locator('article.cb-legal');
    await expect(privacy.getByRole('heading', { name: 'Controller' })).toBeVisible();
    await expect(privacy).toContainText('Art. 28 GDPR');

    await privacy.getByRole('link', { name: 'imprint' }).click();
    await expect(page).toHaveURL(/^[a-z]+:\/\/[^/]+\/imprint$/);
});
