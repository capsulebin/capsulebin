import { fillForm, openCreate, sealedLinks } from './create';
import { catalogue } from './german';
import { expect, test } from './guard';
import { withInstance } from './instance';

/**
 * The create page's expiry select in a real engine: it offers what the
 * instance's `pastes.default_expiry`, `maximum_expiry` and `allow_permanent`
 * accept, with the configured default preselected.
 */

const en = catalogue('en');

const HOUR = 3600;
const DAY = 24 * HOUR;

test('the built-in configuration offers the steps up to 90 days and no "No expiry"', async ({
    page,
}) => {
    await openCreate(page);
    const expiry = page.locator('#create-expiry');
    await expect(expiry.locator('option')).toHaveText([
        en['create.expiry.hour']!,
        en['create.expiry.day']!,
        en['create.expiry.week']!,
        en['create.expiry.30days']!,
        en['create.expiry.90days']!,
    ]);
    await expect(expiry.locator('option[value=""]')).toHaveCount(0);
    await expect(expiry).toHaveValue(String(7 * DAY));
});

test('the select offers the configured default and maximum', async ({ page }) => {
    await withInstance(
        {
            CAPSULEBIN__PASTES__DEFAULT_EXPIRY: '3d',
            CAPSULEBIN__PASTES__MAXIMUM_EXPIRY: '14d',
        },
        async (origin) => {
            await page.goto(`${origin}/`);
            await expect(page.locator('[data-create-form]')).toBeVisible();
            const expiry = page.locator('#create-expiry');
            await expect(expiry.locator('option')).toHaveText([
                en['create.expiry.hour']!,
                en['create.expiry.day']!,
                '3 days',
                en['create.expiry.week']!,
                '14 days',
            ]);
            expect(
                await expiry
                    .locator('option')
                    .evaluateAll((options) =>
                        options.map((option) => (option as HTMLOptionElement).value),
                    ),
            ).toEqual([HOUR, DAY, 3 * DAY, 7 * DAY, 14 * DAY].map(String));
            await expect(expiry).toHaveValue(String(3 * DAY));
            await expect(expiry.locator('option:checked')).toHaveText('3 days');

            // The maximum is accepted by the server, and the sealed page names it.
            await fillForm(page, {
                expiry: String(14 * DAY),
                items: [{ type: 'plain', text: 'twelve bytes' }],
            });
            await page.click('[data-create-submit]');
            await sealedLinks(page);
            await expect(page.locator('[data-summary] li')).toHaveText([
                /^1 item · 12 B$/,
                'Expires in 14 days',
                en['create.summary.noPassword']!,
            ]);
        },
    );
});

test('an instance that keeps capsules forever offers "No expiry" last', async ({ page }) => {
    await withInstance({ CAPSULEBIN__PASTES__ALLOW_PERMANENT: 'true' }, async (origin) => {
        await page.goto(`${origin}/`);
        await expect(page.locator('[data-create-form]')).toBeVisible();
        const expiry = page.locator('#create-expiry');
        await expect(expiry.locator('option')).toHaveCount(6);
        await expect(expiry.locator('option').last()).toHaveText(en['expiry.none']!);
        await expect(expiry.locator('option').last()).toHaveAttribute('value', '');
        await expect(expiry).toHaveValue(String(7 * DAY));
    });
});
