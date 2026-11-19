import { test, expect } from '@playwright/test';
import { installGuard } from './guard';

/**
 * The guard has to notice what it is meant to catch, or every other test here
 * passes for the wrong reason. This provokes each kind of violation once on a
 * real page and expects the guard to have recorded it.
 */
test('the guard records CSP, Trusted Types and XSS canary violations', async ({ context }) => {
    const guard = await installGuard(context);
    const page = await context.newPage();
    await page.goto('/en/');
    await expect(page.locator('[data-create-form]')).toBeVisible();

    await page.evaluate(() => {
        // An inline style element: style-src.
        try {
            const style = document.createElement('style');
            style.append(document.createTextNode('body {}'));
            document.head.append(style);
        } catch {
            // Reported as a violation either way.
        }
        // An HTML sink without a policy: require-trusted-types-for.
        try {
            document.body.insertAdjacentHTML('beforeend', '<b>untrusted</b>');
        } catch {
            // Expected: Trusted Types throws.
        }
        window.__capsulebinXss?.('self-test');
    });

    await expect.poll(() => guard.problems.join('\n')).toContain('style-src');
    await expect.poll(() => guard.problems.join('\n')).toContain('require-trusted-types-for');
    await expect.poll(() => guard.problems.join('\n')).toContain('XSS canary executed: self-test');
});
