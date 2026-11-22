import { readFileSync } from 'node:fs';
import { create, fill, sealedLinks } from './create';
import { expect, installGuard, test } from './guard';

/**
 * The user-facing flows end to end: create in the browser, open, delete, burn and
 * report. Every test also fails on any CSP or Trusted Types violation, page or
 * console error, or executed XSS canary (see ./guard.ts).
 */

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

    // The bucket refills one attempt every 6 seconds, and each attempt runs
    // PBKDF2 in the browser first, so a slow engine can earn an extra attempt
    // or two on the way: count rejections until the limit answers instead of
    // expecting the 429 at exactly the eleventh.
    const rejected: number[] = [];
    let status = await attempt();
    while (status === 404 && rejected.length < 20) {
        rejected.push(status);
        status = await attempt();
    }
    expect(status).toBe(429);
    expect(rejected.length).toBeGreaterThanOrEqual(10);
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

/**
 * What a create whose answer the browser drops leaves in the console: the
 * engine's note about the failed load and the create page logging the error it
 * handles (Firefox prints that one as just `Error`). Accepted only in the test
 * that drops the answer on purpose.
 */
const DROPPED_CONSOLE =
    /^console error: (Failed to load resource: net::ERR_FAILED$|ApiError: network error\n|Error$)/;

test('"Try again" after a dropped answer creates the capsule once, with the delete link shown before', async ({
    browser,
}) => {
    // A context of its own, so its guard can accept the dropped answer's console lines.
    const context = await browser.newContext();
    const guard = await installGuard(context);
    const page = await context.newPage();

    const createBodies: string[] = [];
    page.on('request', (request) => {
        if (request.url().endsWith('/api/v1/pastes') && request.method() === 'POST') {
            createBodies.push(request.postData() ?? '');
        }
    });
    // The first create reaches the server, which stores the capsule; its answer never
    // reaches the page.
    let dropped = false;
    await page.route('**/api/v1/pastes', async (route) => {
        if (dropped || route.request().method() !== 'POST') return route.continue();
        dropped = true;
        await route.fetch();
        await route.abort();
    });

    await fill(page, { items: [{ type: 'plain', text: 'sent twice, stored once' }] });
    await page.click('[data-create-submit]');

    await expect(page.locator('[data-error-message]')).toHaveText(
        'The server could not be reached. Check your connection and try again.',
        { timeout: 30_000 },
    );
    await expect(page.locator('[data-retry-btn]')).toBeVisible();
    const tryAgain = page.locator('[data-try-again]');
    await expect(tryAgain).toHaveText('Try again');
    await expect(page.locator('[data-unclear]')).toContainText(
        'The capsule may have been created.',
    );
    const shown = (await page.locator('[data-unclear-delete-link]').textContent()) ?? '';
    const id = new URL(shown).pathname.split('/').pop()!;
    expect(shown).toMatch(new RegExp(`/manage/${id}#v1\\.`));

    await tryAgain.click();
    const { share, manage } = await sealedLinks(page);
    // The same request both times: no new encryption, the stored capsule answered as a replay.
    expect(createBodies).toHaveLength(2);
    expect(createBodies[1]).toBe(createBodies[0]);
    expect(manage).toBe(shown);
    expect(new URL(share).pathname).toBe(`/p/${id}`);

    const view = await context.newPage();
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toContainText('sent twice, stored once', {
        timeout: 30_000,
    });

    const managePage = await context.newPage();
    managePage.on('dialog', (dialog) => void dialog.accept());
    await managePage.goto(shown);
    await managePage.click('[data-delete-btn]');
    await expect.poll(() => new URL(managePage.url()).hash, { timeout: 30_000 }).toBe('');
    const gone = await context.newPage();
    await gone.goto(share);
    await expect(gone.locator('h1')).toHaveText('Not available', { timeout: 30_000 });

    await context.close();
    expect(
        guard.problems.filter((problem) => !DROPPED_CONSOLE.test(problem)),
        'the browser reported problems',
    ).toEqual([]);
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

const KEY_REFUSAL =
    'The description or contact contains a link with its key or an invitation token ' +
    '(the part after #). Remove that part and send again — the id in the first field is ' +
    'enough.';

test('the report form refuses a link with its key in the description or contact', async ({
    page,
}) => {
    const { share, manage } = await create(page, {
        items: [{ type: 'plain', text: 'report me' }],
    });
    const reportPosts: string[] = [];
    page.on('request', (request) => {
        if (request.url().endsWith('/api/v1/reports') && request.method() === 'POST') {
            reportPosts.push(request.postData() ?? '');
        }
    });

    await page.goto('/report');
    await page.fill('[data-report-paste]', share);
    await page.fill('[data-report-description]', `Please look at ${share}`);
    await page.click('[data-report-submit]');
    await expect(page.locator('[data-report-error]')).toHaveText(KEY_REFUSAL);
    await expect(page.locator('[data-report-description]')).toBeFocused();
    await expect(page.locator('[data-report-description]')).toHaveValue(`Please look at ${share}`);

    await page.fill('[data-report-description]', 'Browser check of the key refusal.');
    await page.fill('[data-report-contact]', manage);
    await page.click('[data-report-submit]');
    await expect(page.locator('[data-report-error]')).toHaveText(KEY_REFUSAL);
    await expect(page.locator('[data-report-contact]')).toBeFocused();
    await expect(page.locator('[data-report-contact]')).toHaveValue(manage);

    // A request would have been sent right after the click; give it time to show.
    await page.waitForTimeout(500);
    expect(reportPosts).toEqual([]);
});

test('the report form refuses an invitation link in the description or contact', async ({
    page,
}) => {
    const { share } = await create(page, {
        items: [{ type: 'plain', text: 'report me too' }],
    });
    const reportPosts: string[] = [];
    page.on('request', (request) => {
        if (request.url().endsWith('/api/v1/reports') && request.method() === 'POST') {
            reportPosts.push(request.postData() ?? '');
        }
    });
    const origin = new URL(share).origin;
    const invitation = `${origin}/#token=e2e-invitation-token`;
    const invitationDe = `${origin}/de/#token=e2e-invitation-token`;

    await page.goto('/report');
    await page.fill('[data-report-paste]', share);
    await page.fill('[data-report-description]', `Invited with\n${invitation}`);
    await page.click('[data-report-submit]');
    await expect(page.locator('[data-report-error]')).toHaveText(KEY_REFUSAL);
    await expect(page.locator('[data-report-description]')).toBeFocused();
    await expect(page.locator('[data-report-description]')).toHaveValue(
        `Invited with\n${invitation}`,
    );

    await page.fill('[data-report-description]', 'Browser check of the token refusal.');
    await page.fill('[data-report-contact]', invitationDe);
    await page.click('[data-report-submit]');
    await expect(page.locator('[data-report-error]')).toHaveText(KEY_REFUSAL);
    await expect(page.locator('[data-report-contact]')).toBeFocused();
    await expect(page.locator('[data-report-contact]')).toHaveValue(invitationDe);

    // A request would have been sent right after the click; give it time to show.
    await page.waitForTimeout(500);
    expect(reportPosts).toEqual([]);
});

test('a password of only spaces is refused before anything is sent', async ({ page, context }) => {
    const createPosts: string[] = [];
    page.on('request', (request) => {
        if (request.url().endsWith('/api/v1/pastes') && request.method() === 'POST') {
            createPosts.push(request.url());
        }
    });
    await page.goto('/');
    await page.fill('[data-item-text]', 'protected by a real password');
    await page.fill('#create-password', '   ');
    await page.click('[data-create-submit]');

    const message = page.locator('[data-password-error]');
    await expect(message).toHaveText(
        'The password consists only of spaces. Enter a real password, or leave the field ' +
            'empty for no password.',
    );
    await expect(page.locator('#create-password')).toBeFocused();
    await expect(page.locator('[data-item-text]')).toHaveValue('protected by a real password');
    await page.waitForTimeout(500);
    expect(createPosts).toEqual([]);

    const password = 'the real one';
    await page.fill('#create-password', password);
    await page.click('[data-create-submit]');
    const { share } = await sealedLinks(page);
    expect(createPosts).toHaveLength(1);

    const view = await context.newPage();
    await view.goto(share);
    await view.fill('#pwd-input', '   ');
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-error')).toHaveText(
        'Wrong password, or the capsule is no longer available.',
        { timeout: 30_000 },
    );
    await view.fill('#pwd-input', password);
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-item-panel')).toContainText('protected by a real password', {
        timeout: 30_000,
    });
});

// The password field belongs to a form of its own (so password managers do not
// take the item name for a username); Enter in it still creates the capsule once.
test('Enter in the password field creates the capsule', async ({ page, context }) => {
    const createPosts: string[] = [];
    page.on('request', (request) => {
        if (request.url().endsWith('/api/v1/pastes') && request.method() === 'POST') {
            createPosts.push(request.url());
        }
    });
    await page.goto('/');
    await page.fill('[data-item-text]', 'sent with Enter');
    await page.fill('#create-item-name', 'notes.txt');
    await page.fill('#create-password', 'enter key');
    expect(
        await page.locator('#create-password').evaluate((input: HTMLInputElement) => ({
            own: input.form?.id,
            textFields: [...(input.form?.elements ?? [])].filter((e) => e !== input).length,
        })),
    ).toEqual({ own: 'create-password-form', textFields: 0 });
    await page.press('#create-password', 'Enter');
    const { share } = await sealedLinks(page);
    expect(createPosts).toHaveLength(1);

    const view = await context.newPage();
    await view.goto(share);
    await view.fill('#pwd-input', 'enter key');
    await view.click('button[type=submit]');
    await expect(view.locator('.cb-item-panel')).toContainText('sent with Enter', {
        timeout: 30_000,
    });
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
