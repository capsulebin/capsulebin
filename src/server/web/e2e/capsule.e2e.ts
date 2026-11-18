import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { expect, test } from './guard';

/**
 * The user-facing flows end to end: create in the browser, open, delete, burn and
 * report. Every test also fails on any CSP or Trusted Types violation, page or
 * console error, or executed XSS canary (see ./guard.ts).
 */

type Item =
    | { type: 'plain' | 'markdown'; name?: string; text: string }
    | { type: 'code'; name?: string; lang: string; text: string }
    | { type: 'download'; name: string; bytes: number[] };

interface Capsule {
    title?: string;
    items: Item[];
    password?: string;
    burn?: boolean;
}

/** Creates a capsule through the create page and returns its two links. */
async function create(page: Page, capsule: Capsule): Promise<{ share: string; manage: string }> {
    await page.goto('/');
    await expect(page.locator('[data-create-form]')).toBeVisible();
    if (capsule.title) await page.fill('#create-title', capsule.title);

    for (const [index, item] of capsule.items.entries()) {
        if (index > 0) {
            await page.click('[data-item-add]');
            await page.click(`[data-add-type="${item.type}"]`);
        } else if (item.type !== 'plain') {
            await page.selectOption('#create-item-type', item.type);
        }
        if (item.name) await page.fill('#create-item-name', item.name);
        if (item.type === 'code') await page.selectOption('#create-item-lang', item.lang);
        if (item.type === 'download') {
            await page.setInputFiles('[data-item-file]', {
                name: item.name,
                mimeType: 'application/octet-stream',
                buffer: Buffer.from(item.bytes),
            });
        } else {
            await page.fill('[data-item-text]', item.text);
        }
    }
    if (capsule.password) await page.fill('#create-password', capsule.password);
    if (capsule.burn) await page.check('#create-burn');

    await page.click('[data-create-submit]');
    const share = page.locator('#share-link');
    await expect(share).not.toHaveValue('', { timeout: 30_000 });
    return { share: await share.inputValue(), manage: await page.inputValue('#delete-link') };
}

const MARKDOWN = [
    '# Heading',
    '',
    '**bold** and a [link](https://example.org).',
    '',
    '```rust',
    'fn main() {}',
    '```',
].join('\n');

/** Raw HTML and links that would run the XSS canary if anything got through. */
const HOSTILE_MARKDOWN = [
    '# Hostile',
    '',
    '<img src="x" onerror="__capsulebinXss(\'img onerror\')">',
    '<script>__capsulebinXss("script element")</script>',
    '<a href="javascript:__capsulebinXss(\'raw link\')">raw link</a>',
    '[markdown link](javascript:__capsulebinXss("markdown link"))',
    '<details><summary>allowed</summary>kept</details>',
].join('\n');

test('a capsule with four items and a password', async ({ page, context }) => {
    const password = 'correct horse battery staple';
    const { share } = await create(page, {
        title: 'Browser check',
        password,
        items: [
            { type: 'plain', name: 'notes.txt', text: 'plain text <b>not bold</b>' },
            { type: 'markdown', name: 'readme.md', text: MARKDOWN },
            { type: 'code', name: 'main.rs', lang: 'rust', text: 'fn main() { let x = 1; }' },
            { type: 'download', name: 'data.bin', bytes: [0, 1, 2, 3, 255] },
        ],
    });
    await page.click('[data-theme-toggle]');

    const view = await context.newPage();
    await view.goto(share);

    // The server answers a wrong password like an unknown capsule (ADR-001); the
    // prompt stays, saying it cannot tell the two apart.
    await view.fill('#pwd-input', 'wrong password');
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-error')).toHaveText(
        'Wrong password, or the capsule is no longer available.',
        { timeout: 30_000 },
    );
    await view.fill('#pwd-input', password);
    await view.click('button[type=submit]');
    const tabs = view.locator('.cb-itemtab');
    await expect(tabs).toHaveCount(4, { timeout: 30_000 });
    await expect(view.locator('h1')).toHaveText('Browser check');
    const panel = view.locator('.cb-item-panel');

    // Text is text, never markup.
    await tabs.nth(0).click();
    await expect(panel).toContainText('plain text <b>not bold</b>');

    // Markdown is rendered.
    await tabs.nth(1).click();
    await expect(panel.locator('h1').first()).toHaveText('Heading');
    await expect(panel.locator('strong')).toHaveText('bold');
    await expect(panel.locator('pre code')).toContainText('fn main() {}');

    // Code is highlighted.
    await tabs.nth(2).click();
    await expect(panel.locator('[class*="hljs-"]').first()).toBeVisible();

    // A file comes back byte for byte.
    await tabs.nth(3).click();
    const [download] = await Promise.all([
        view.waitForEvent('download'),
        panel
            .getByRole('button', { name: /download/i })
            .first()
            .click(),
    ]);
    expect(download.suggestedFilename()).toBe('data.bin');
    expect([...readFileSync(await download.path())]).toEqual([0, 1, 2, 3, 255]);

    await view.click('[data-theme-toggle]');
});

test('too many wrong passwords are throttled with a wait', async ({ page, context }) => {
    // invalid_capability_per_paste is on by default: 10 per minute, burst 10.
    const { share } = await create(page, {
        password: 'the right one',
        items: [{ type: 'plain', text: 'throttled' }],
    });
    const view = await context.newPage();
    await view.goto(share);

    const attempt = async () => {
        await view.fill('#pwd-input', 'wrong');
        const [response] = await Promise.all([
            view.waitForResponse((r) => r.url().endsWith('/retrieve'), { timeout: 30_000 }),
            view.click('button[type=submit]'),
        ]);
        await expect(view.locator('#pwd-input')).toBeEnabled({ timeout: 30_000 });
        return response.status();
    };

    for (let i = 0; i < 10; i += 1) expect(await attempt()).toBe(404);
    expect(await attempt()).toBe(429);
    await expect(view.locator('.cb-error')).toHaveText(
        /^Too many attempts\. Try again in \d+ seconds?\.$/,
    );
});

test('the management link deletes a capsule', async ({ page, context }) => {
    const { share, manage } = await create(page, { items: [{ type: 'plain', text: 'delete me' }] });

    const managePage = await context.newPage();
    managePage.on('dialog', (dialog) => void dialog.accept());
    await managePage.goto(manage);
    await managePage.click('[data-delete-btn]');
    // The delete secret leaves the address bar once the capsule is gone.
    await expect.poll(() => new URL(managePage.url()).hash, { timeout: 30_000 }).toBe('');

    const view = await context.newPage();
    await view.goto(share);
    await expect(view.locator('h1')).toHaveText('Not available', { timeout: 30_000 });
});

test('a burn capsule opens exactly once', async ({ page, context }) => {
    const { share } = await create(page, {
        items: [{ type: 'plain', text: 'burn me' }],
        burn: true,
    });

    const first = await context.newPage();
    await first.goto(share);
    await first.getByRole('button', { name: 'Yes, open and burn' }).click();
    await expect(first.locator('.cb-item-panel')).toContainText('burn me', { timeout: 30_000 });

    const second = await context.newPage();
    await second.goto(share);
    await expect(second.locator('h1')).toHaveText(/Not available|Already retrieved/, {
        timeout: 30_000,
    });
});

test('a burn capsule with a password survives a wrong password', async ({ page, context }) => {
    const { share } = await create(page, {
        password: 'the right one',
        burn: true,
        items: [{ type: 'plain', text: 'burn after the right password' }],
    });
    const view = await context.newPage();
    await view.goto(share);
    await view.getByRole('button', { name: 'Yes, open and burn' }).click();

    // A wrong password consumes nothing: the server checks the reveal token first.
    await view.fill('#pwd-input', 'wrong');
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-error')).toHaveText(
        'Wrong password, or the capsule is no longer available.',
        { timeout: 30_000 },
    );

    // The burn warning was confirmed once and is not asked again.
    await expect(view.getByRole('button', { name: 'Yes, open and burn' })).toHaveCount(0);
    await view.fill('#pwd-input', 'the right one');
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-item-panel')).toContainText('burn after the right password', {
        timeout: 30_000,
    });
});

test('the report form sends the id without the key', async ({ page }) => {
    const { share } = await create(page, { items: [{ type: 'plain', text: 'report me' }] });

    await page.goto('/report');
    await page.fill('[data-report-paste]', share);
    await page.selectOption('[data-report-reason]', 'spam');
    await page.fill('[data-report-description]', 'Browser check of the report form.');
    const [response] = await Promise.all([
        page.waitForResponse(
            (r) => r.url().endsWith('/api/v1/reports') && r.request().method() === 'POST',
        ),
        page.click('[data-report-submit]'),
    ]);
    expect(response.status()).toBe(202);
    const sent = response.request().postDataJSON() as { paste_id: string };
    expect(share).toContain(`/p/${sent.paste_id}#`);
    expect(JSON.stringify(sent)).not.toContain('#');
});

test('hostile HTML in Markdown is removed and never runs', async ({
    page,
    context,
    guard,
    browserName,
}) => {
    // DOMPurify parses the raw HTML in an inert document and only then strips the
    // handler. Chromium reports the onerror attribute it sees during that parse as
    // a script-src-attr violation, although nothing in that document can run.
    // Whether anything did run is what the XSS canary and the checks below decide.
    if (browserName === 'chromium') guard.allow('script-src-attr', /\/assets\/render-markdown-/);

    const { share } = await create(page, { items: [{ type: 'markdown', text: HOSTILE_MARKDOWN }] });
    const view = await context.newPage();
    await view.goto(share);
    const panel = view.locator('.cb-item-panel');
    await expect(panel.locator('h1')).toHaveText('Hostile', { timeout: 30_000 });

    await expect(panel.locator('img, script, [onerror]')).toHaveCount(0);
    await expect(panel.locator('a[href^="javascript:"]')).toHaveCount(0);
    await expect(panel.locator('details summary')).toHaveText('allowed');
});
