import { expect, test } from './guard';

/**
 * The operator's legal pages (`[legal]`, set in playwright.config.ts): linked
 * from the footer of every page and served by the server into `legal.html`,
 * under the same CSP as everything else. The guard fails the test on any
 * violation or console error.
 */

const LINKS = [
    ['Imprint', '/imprint'],
    ['Privacy', '/privacy'],
    ['Terms', '/terms'],
] as const;

for (const path of [
    '/',
    '/p/2pS3xQ7Rk9mNpLz4AbCdEA',
    '/manage/2pS3xQ7Rk9mNpLz4AbCdEA',
    '/report',
    '/imprint',
]) {
    test(`the footer of ${path} links to the legal pages`, async ({ page }) => {
        await page.goto(path);

        const footer = page.locator('footer');
        for (const [name, href] of LINKS) {
            await expect(footer.getByRole('link', { name, exact: true })).toHaveAttribute(
                'href',
                href,
            );
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
    await expect(page).toHaveURL(/\/privacy$/);
    const privacy = page.locator('article.cb-legal');
    await expect(privacy.getByRole('heading', { name: 'Controller' })).toBeVisible();
    await expect(privacy).toContainText('Art. 28 GDPR');

    await privacy.getByRole('link', { name: 'imprint' }).click();
    await expect(page).toHaveURL(/\/imprint$/);
});
