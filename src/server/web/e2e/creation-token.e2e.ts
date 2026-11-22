import {
    type Browser,
    type BrowserContext,
    type Page,
    type Request,
    test as base,
} from '@playwright/test';
import { create, sealedLinks } from './create';
import { expectGerman } from './german';
import { type Guard, expect, installGuard } from './guard';
import { creationToken, withInstance } from './instance';

/**
 * The creation token on the create page (ADR-018) in a real engine: the page per
 * creation mode, the token field and "Remember", the invitation link
 * `#token=<token>`, "Forget", and a refused token that keeps the draft and sends
 * the same sealed capsule again.
 *
 * The main instance of playwright.config.ts is in `public` mode. Per worker one
 * instance in `token` mode, with a token from `capsulebin token`, and one in
 * `disabled` mode are started. Every browser context runs under the guard.
 */

const KEY = 'capsulebin.creation_token';

/**
 * What a refused create leaves in the console: the engine's note about the
 * failed `401` load, and the create page logging the error it handles (Firefox
 * prints that one as just `Error`). Accepted only in tests that send a wrong
 * token on purpose ({@link test}'s `refusals`).
 */
const REFUSAL_CONSOLE = /^console error: (Error$|.*\b401\b|.*Unauthorized)/;

interface TokenInstance {
    origin: string;
    token: string;
}

const test = base.extend<
    { guard: Guard; refusals: boolean },
    { tokenInstance: TokenInstance; disabledOrigin: string }
>({
    refusals: [false, { option: true }],
    guard: [
        async ({ context, refusals }, use) => {
            const guard = await installGuard(context);
            await use(guard);
            expect(problems(guard, refusals), 'the browser reported problems').toEqual([]);
        },
        { auto: true },
    ],
    tokenInstance: [
        async ({}, use) => {
            const { token, hash } = creationToken();
            await withInstance(
                {
                    CAPSULEBIN__CREATION__MODE: 'token',
                    CAPSULEBIN__CREATION__TOKEN_HASHES: `["${hash}"]`,
                },
                (origin) => use({ origin, token }),
            );
        },
        { scope: 'worker' },
    ],
    disabledOrigin: [
        async ({}, use) => withInstance({ CAPSULEBIN__CREATION__MODE: 'disabled' }, use),
        { scope: 'worker' },
    ],
});

function problems(guard: Guard, refusals: boolean): string[] {
    return guard.problems.filter((problem) => !(refusals && REFUSAL_CONSOLE.test(problem)));
}

/** A fresh browser under its own guard, for a page that must start with nothing stored. */
async function freshBrowser(
    browser: Browser,
    locale = 'en-US',
): Promise<{ context: BrowserContext; page: Page; guard: Guard }> {
    const context = await browser.newContext({ locale });
    const guard = await installGuard(context);
    return { context, guard, page: await context.newPage() };
}

async function stored(page: Page): Promise<string | null> {
    return page.evaluate((key) => localStorage.getItem(key), KEY);
}

interface Sent {
    method: string;
    url: string;
    authorization: string | undefined;
    body: string | null;
}

/** Every request of `context` from now on, with its complete headers and body. */
function recordRequests(context: BrowserContext): () => Promise<Sent[]> {
    const pending: Promise<Sent>[] = [];
    context.on('request', (request: Request) => {
        pending.push(
            request.allHeaders().then((headers) => ({
                method: request.method(),
                url: request.url(),
                authorization: headers['authorization'],
                body: request.postData(),
            })),
        );
    });
    return () => Promise.all(pending);
}

const isCreate = (sent: Pick<Sent, 'method' | 'url'>) =>
    sent.method === 'POST' && new URL(sent.url).pathname === '/api/v1/pastes';

/** Enters `token` into the token field, with "Remember" as given. */
async function enterToken(page: Page, token: string, remember = true): Promise<void> {
    await page.fill('#creation-token', token);
    await page.setChecked('[data-token-remember]', remember);
    await page.click('[data-token-submit]');
}

/** The draft of the refused-token flows: two items, the second one active. */
const DRAFT = {
    title: 'Two items under one token',
    first: 'first item, plain text',
    second: '# second item\n\nin Markdown',
    password: 'draft password',
    expiry: '86400',
} as const;

async function writeDraft(page: Page): Promise<void> {
    await expect(page.locator('[data-create-form]')).toBeVisible();
    await page.fill('#create-title', DRAFT.title);
    await page.fill('[data-item-text]', DRAFT.first);
    await page.click('[data-item-add]');
    await page.click('[data-add-type="markdown"]');
    await page.fill('[data-item-text]', DRAFT.second);
    await page.selectOption('#create-expiry', DRAFT.expiry);
    await page.fill('#create-password', DRAFT.password);
}

/** Submits whatever form is shown (the draft or the token prompt) and waits for the create's answer. */
async function submitCreate(page: Page, button: string): Promise<number> {
    const [response] = await Promise.all([
        page.waitForResponse((r) => isCreate({ method: r.request().method(), url: r.url() }), {
            timeout: 30_000,
        }),
        page.click(button),
    ]);
    return response.status();
}

test.describe('token mode', () => {
    test('without a token the page asks for one and checks it before use', async ({
        page,
        tokenInstance,
    }) => {
        await page.goto(`${tokenInstance.origin}/`);
        await expect(page.locator('.cb-lede')).toHaveText(
            'This server creates capsules only with a creation token. Its operator can give you one, or an invitation link that stores it in this browser.',
        );
        await expect(page.locator('#creation-token')).toBeVisible();
        await expect(page.locator('[data-token-remember]')).toBeChecked();
        await expect(page.locator('[data-create-form]')).toHaveCount(0);

        // A space inside, or more than 1024 characters: refused inline, nothing held.
        for (const invalid of ['abc def', 'x'.repeat(1025)]) {
            await enterToken(page, invalid);
            await expect(page.locator('#creation-token-error')).toHaveText(
                'This is not a creation token. A token has 1 to 1024 characters: letters, digits and punctuation, no spaces.',
            );
            await expect(page.locator('#creation-token')).toHaveAttribute('aria-invalid', 'true');
            await expect(page.locator('[data-create-form]')).toHaveCount(0);
            expect(await stored(page)).toBeNull();
        }

        // Surrounding whitespace is trimmed.
        await enterToken(page, `  ${tokenInstance.token}  `);
        await expect(page.locator('[data-create-form]')).toBeVisible();
        await expect(page.locator('[data-token-line]')).toHaveText(
            /^\s*Token stored in this browser\s*·\s*Forget\s*$/,
        );
        expect(await stored(page)).toBe(tokenInstance.token);
    });

    test('the token is the bearer of the create request and of nothing else', async ({
        page,
        context,
        tokenInstance,
    }) => {
        const requests = recordRequests(context);
        await page.goto(`${tokenInstance.origin}/`);
        await enterToken(page, tokenInstance.token);

        const password = 'request listener';
        await page.fill('[data-item-text]', 'sent with a bearer');
        await page.fill('#create-password', password);
        expect(await submitCreate(page, '[data-create-submit]')).toBe(201);
        const { share, manage } = await sealedLinks(page);

        const view = await context.newPage();
        await view.goto(share);
        await view.fill('#pwd-input', password);
        await view.click('button[type=submit]');
        await expect(view.locator('.cb-item-panel')).toContainText('sent with a bearer', {
            timeout: 30_000,
        });

        const managePage = await context.newPage();
        managePage.on('dialog', (dialog) => void dialog.accept());
        await managePage.goto(manage);
        await managePage.click('[data-delete-btn]');
        await expect.poll(() => new URL(managePage.url()).hash, { timeout: 30_000 }).toBe('');

        const sent = await requests();
        const api = sent.filter((r) => new URL(r.url).pathname.startsWith('/api/'));
        const creates = api.filter(isCreate);
        expect(creates.map((r) => r.authorization)).toEqual([`Bearer ${tokenInstance.token}`]);
        const others = api.filter((r) => !isCreate(r));
        // Describe and retrieve of the view page, describe and delete of the manage page.
        for (const path of ['/describe', '/retrieve']) {
            expect(others.some((r) => r.url.endsWith(path))).toBe(true);
        }
        expect(others.some((r) => r.method === 'DELETE')).toBe(true);
        expect(others.filter((r) => r.authorization !== undefined)).toEqual([]);
        expect(
            sent.filter(
                (r) => r.url.includes(tokenInstance.token) || r.body?.includes(tokenInstance.token),
            ),
        ).toEqual([]);
    });

    test('"Remember" unchecked holds the token for this page only', async ({
        page,
        context,
        tokenInstance,
    }) => {
        const requests = recordRequests(context);
        await page.goto(`${tokenInstance.origin}/`);
        await enterToken(page, tokenInstance.token, false);
        await expect(page.locator('[data-token-line]')).toHaveText(
            /^\s*Token for this page only\s*·\s*Forget\s*$/,
        );
        expect(await stored(page)).toBeNull();

        await page.fill('[data-item-text]', 'created with a page-only token');
        expect(await submitCreate(page, '[data-create-submit]')).toBe(201);
        await sealedLinks(page);
        expect((await requests()).filter(isCreate).map((r) => r.authorization)).toEqual([
            `Bearer ${tokenInstance.token}`,
        ]);
        expect(await stored(page)).toBeNull();

        await page.reload();
        await expect(page.locator('#creation-token')).toBeVisible();
        await expect(page.locator('[data-create-form]')).toHaveCount(0);
        expect(await stored(page)).toBeNull();
    });

    test('"Forget" removes the stored token and asks for one again', async ({
        page,
        tokenInstance,
    }) => {
        await page.goto(`${tokenInstance.origin}/`);
        await enterToken(page, tokenInstance.token);
        expect(await stored(page)).toBe(tokenInstance.token);

        await page.reload();
        await expect(page.locator('[data-create-form]')).toBeVisible();
        await page.click('[data-token-forget]');
        await expect(page.locator('#creation-token')).toBeVisible();
        await expect(page.locator('[data-create-form]')).toHaveCount(0);
        expect(await stored(page)).toBeNull();

        await page.reload();
        await expect(page.locator('#creation-token')).toBeVisible();
    });

    test.describe('a refused token', () => {
        test.use({ refusals: true });

        test('is forgotten, and the correct one sends the same capsule', async ({
            page,
            context,
            tokenInstance,
        }) => {
            const requests = recordRequests(context);
            await page.goto(`${tokenInstance.origin}/`);
            await enterToken(page, 'wrong-token');
            await writeDraft(page);

            expect(await submitCreate(page, '[data-create-submit]')).toBe(401);
            await expect(page.locator('[data-error-message]')).toHaveText(
                'The server did not accept the creation token. Enter a valid token to create the capsule; your content stays encrypted as it is.',
            );
            expect(await stored(page)).toBeNull();
            await expect(page.locator('#creation-token')).toBeVisible();
            await expect(page.locator('[data-token-submit]')).toHaveText('Create');

            // A second wrong token: the prompt again.
            await page.fill('#creation-token', 'still-wrong');
            expect(await submitCreate(page, '[data-token-submit]')).toBe(401);
            await expect(page.locator('[data-error-message]')).toContainText(
                'The server did not accept the creation token.',
            );
            expect(await stored(page)).toBeNull();

            await page.fill('#creation-token', tokenInstance.token);
            expect(await submitCreate(page, '[data-token-submit]')).toBe(201);
            const { share } = await sealedLinks(page);
            expect(await stored(page)).toBe(tokenInstance.token);

            // No new encryption: the very same request body each time, so the same id.
            const creates = (await requests()).filter(isCreate);
            expect(creates.map((r) => r.authorization)).toEqual([
                'Bearer wrong-token',
                'Bearer still-wrong',
                `Bearer ${tokenInstance.token}`,
            ]);
            const ids = creates.map((r) => (JSON.parse(r.body ?? '{}') as { id?: string }).id);
            expect(ids[0]).toBeTruthy();
            expect(ids).toEqual([ids[0], ids[0], ids[0]]);
            expect(new Set(creates.map((r) => r.body)).size).toBe(1);
            expect(share).toContain(`/p/${ids[0]}#`);

            const view = await context.newPage();
            await view.goto(share);
            await view.fill('#pwd-input', DRAFT.password);
            await view.click('button[type=submit]');
            const tabs = view.locator('.cb-itemtab');
            await expect(tabs).toHaveCount(2, { timeout: 30_000 });
            await expect(view.locator('h1')).toHaveText(DRAFT.title);
            const panel = view.locator('.cb-item-panel');
            await tabs.nth(0).click();
            await expect(panel).toContainText(DRAFT.first);
            await tabs.nth(1).click();
            await expect(panel.locator('h1')).toHaveText('second item');
        });

        test('"Back to editing" brings the draft back as it was submitted', async ({
            page,
            tokenInstance,
        }) => {
            await page.goto(`${tokenInstance.origin}/`);
            await enterToken(page, 'wrong-token');
            await writeDraft(page);
            await page.check('#create-burn');
            expect(await submitCreate(page, '[data-create-submit]')).toBe(401);

            await page.click('[data-retry-btn]');
            await expect(page.locator('[data-create-form]')).toBeVisible();
            await expect(page.locator('#create-title')).toHaveValue(DRAFT.title);
            await expect(page.locator('#create-expiry')).toHaveValue(DRAFT.expiry);
            await expect(page.locator('#create-burn')).toBeChecked();
            await expect(page.locator('#create-password')).toHaveValue(DRAFT.password);
            const tabs = page.locator('.cb-itemtab');
            await expect(tabs).toHaveCount(2);
            // The active item is still the second one.
            await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
            await expect(page.locator('#create-item-type')).toHaveValue('markdown');
            await expect(page.locator('[data-item-text]')).toHaveValue(DRAFT.second);
            await tabs.nth(0).click();
            await expect(page.locator('#create-item-type')).toHaveValue('plain');
            await expect(page.locator('[data-item-text]')).toHaveValue(DRAFT.first);
        });
    });
});

test.describe('the invitation link', () => {
    const accepted = 'This browser can now create capsules.';

    for (const [path, locale, confirmation] of [
        ['/', 'en-US', accepted],
        ['/en/', 'en-US', accepted],
        ['/de/', 'de-DE', 'Dieser Browser kann jetzt Kapseln erstellen.'],
    ] as const) {
        test(`${path}#token= stores the token and leaves the address`, async ({
            browser,
            tokenInstance,
        }) => {
            const { context, page, guard } = await freshBrowser(browser, locale);
            await page.goto(`${tokenInstance.origin}${path}#token=${tokenInstance.token}`);
            await expect(page.locator('[data-token-notice]')).toHaveText(confirmation);
            await expect(page.locator('[data-create-form]')).toBeVisible();
            expect(await page.evaluate(() => location.hash)).toBe('');
            expect(page.url()).not.toContain('#');
            expect(page.url()).not.toContain(tokenInstance.token);
            expect(await stored(page)).toBe(tokenInstance.token);

            // The history entry holds no fragment either.
            const landed = page.url();
            await page.goto(`${tokenInstance.origin}/report`);
            await page.goBack();
            expect(page.url()).toBe(landed);

            await context.close();
            expect(guard.problems).toEqual([]);
        });
    }

    test('with an unusable token stores nothing and says so', async ({
        browser,
        tokenInstance,
    }) => {
        for (const invalid of ['', 'x'.repeat(1025)]) {
            const { context, page, guard } = await freshBrowser(browser);
            await page.goto(`${tokenInstance.origin}/#token=${invalid}`);
            await expect(page.locator('[data-token-notice]')).toHaveText(
                'This invitation link cannot be used. Ask the operator for a new one.',
            );
            await expect(page.locator('#creation-token')).toBeVisible();
            await expect(page.locator('[data-create-form]')).toHaveCount(0);
            expect(await page.evaluate(() => location.hash)).toBe('');
            expect(await stored(page)).toBeNull();
            await context.close();
            expect(guard.problems).toEqual([]);
        }
    });

    test('any other fragment is left alone', async ({ page, tokenInstance }) => {
        await page.goto(`${tokenInstance.origin}/#tokens=${tokenInstance.token}`);
        await expect(page.locator('#creation-token')).toBeVisible();
        expect(await page.evaluate(() => location.hash)).toBe(`#tokens=${tokenInstance.token}`);
        await expect(page.locator('[data-token-notice]')).toBeHidden();
        expect(await stored(page)).toBeNull();
    });

    test('is removed and not stored on a public instance', async ({ page }) => {
        await page.goto('/#token=some-token');
        await expect(page.locator('[data-create-form]')).toBeVisible();
        expect(await page.evaluate(() => location.hash)).toBe('');
        expect(page.url()).not.toContain('#');
        await expect(page.locator('[data-token-notice]')).toBeHidden();
        expect(await stored(page)).toBeNull();
    });

    test('is removed and not stored on a disabled instance', async ({ page, disabledOrigin }) => {
        await page.goto(`${disabledOrigin}/#token=some-token`);
        await expect(page.locator('h1')).toHaveText('No new capsules');
        expect(await page.evaluate(() => location.hash)).toBe('');
        expect(page.url()).not.toContain('#');
        expect(await stored(page)).toBeNull();
    });
});

test('public mode sends no Authorization header and shows no token UI', async ({
    page,
    context,
}) => {
    const requests = recordRequests(context);
    await create(page, { items: [{ type: 'plain', text: 'public instance' }] });
    const creates = (await requests()).filter(isCreate);
    expect(creates).toHaveLength(1);
    expect(creates.map((r) => r.authorization)).toEqual([undefined]);

    await page.goto('/');
    await expect(page.locator('[data-create-form]')).toBeVisible();
    await expect(page.locator('[data-token-line]')).toBeHidden();
    await expect(page.locator('#creation-token')).toHaveCount(0);
});

test('disabled mode shows only the notice', async ({ page, disabledOrigin }) => {
    await page.goto(`${disabledOrigin}/`);
    await expect(page.locator('h1')).toHaveText('No new capsules');
    await expect(page.locator('.cb-lede')).toHaveText(
        'This server accepts no new capsules. Existing links keep working.',
    );
    await expect(page.locator('[data-create-form]')).toHaveCount(0);
    await expect(page.locator('#creation-token')).toHaveCount(0);
    await expect(page.locator('form')).toHaveCount(0);
});

test.describe('in a German browser', () => {
    test.use({ locale: 'de-DE', refusals: true });

    test('the token prompt and a refused token stay German', async ({ page, tokenInstance }) => {
        await page.goto(`${tokenInstance.origin}/de/`);
        await expectGerman(page, 'Dieser Server erstellt Kapseln nur mit einem Erstellungstoken.');
        await expect(page.locator('[data-token-remember]')).toBeChecked();

        await enterToken(page, 'kein Token');
        await expectGerman(page, 'Das ist kein Erstellungstoken.');

        await enterToken(page, 'falsches-token');
        await expectGerman(page, 'Token in diesem Browser gespeichert');
        await writeDraft(page);
        expect(await submitCreate(page, '[data-create-submit]')).toBe(401);
        await expectGerman(page, 'Der Server hat das Erstellungstoken nicht angenommen.');
        await expect(page.locator('[data-token-submit]')).toHaveText('Erstellen');

        await page.fill('#creation-token', tokenInstance.token);
        expect(await submitCreate(page, '[data-token-submit]')).toBe(201);
        await expectGerman(page, 'Versiegelt.');
    });
});
