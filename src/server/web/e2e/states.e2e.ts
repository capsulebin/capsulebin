import type { BrowserContext, Locator, Page, Route } from '@playwright/test';
import { create, fill, fillForm, sealedLinks } from './create';
import { catalogue } from './german';
import { expect, installGuard, test } from './guard';
import { withInstance } from './instance';
import { colour, elementsWith, settled, token } from './style';

/**
 * The sealed page and every state page in a real engine: the hexagon of each
 * state (`data-hexagon`), its animations with and without reduced motion, the
 * share link with its key, the summary, the copy buttons' "Copied" state, and
 * the flows through opening, decrypting, the password prompt and the burn
 * warning.
 */

const en = catalogue('en');

/**
 * A request matching `url` (and `method`) held until `release` is called, then
 * passed on to the server, or answered by `answer`.
 */
async function hold(
    page: Page,
    url: string,
    method = 'POST',
    answer: (route: Route) => Promise<void> = (route) => route.fallback(),
): Promise<{ release: () => void; reached: Promise<void> }> {
    let release!: () => void;
    let reached!: () => void;
    const released = new Promise<void>((done) => (release = done));
    const arrived = new Promise<void>((done) => (reached = done));
    await page.route(url, async (route: Route) => {
        if (route.request().method() !== method) return route.fallback();
        reached();
        await released;
        await answer(route);
    });
    return { release, reached: arrived };
}

const ring = (hexagon: Locator) => hexagon.locator('.cb-hex-ring');

/** Name, duration in seconds and iteration count of the ring's animation. */
async function ringAnimation(
    hexagon: Locator,
): Promise<{ name: string; duration: number; iterations: string; dashes: string }> {
    return ring(hexagon).evaluate((element) => {
        const style = getComputedStyle(element);
        return {
            name: style.animationName,
            duration: Number.parseFloat(style.animationDuration),
            iterations: style.animationIterationCount,
            dashes: style.strokeDasharray,
        };
    });
}

async function expectState(page: Page, state: string, heading: string): Promise<Locator> {
    const hexagon = page.locator(`main [data-hexagon="${state}"]`);
    await expect(hexagon).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('main h1')).toHaveText(heading);
    return hexagon;
}

async function expectRunning(hexagon: Locator): Promise<void> {
    const animation = await ringAnimation(hexagon);
    expect(animation.name).not.toBe('none');
    expect(animation.iterations).toBe('infinite');
}

async function expectEmpty(hexagon: Locator): Promise<void> {
    expect((await ringAnimation(hexagon)).dashes).not.toBe('none');
}

async function expectWarning(page: Page, hexagon: Locator): Promise<void> {
    expect(await colour(ring(hexagon), 'stroke')).toBe(await token(page, '--cb-warning'));
}

/**
 * Lets the page write to the clipboard in Chromium, the only engine that asks for these
 * permissions. Firefox and WebKit allow writing on a click; WebKit accepts the grant but then
 * refuses to open a page in the context ("Unknown permission: clipboard-write").
 */
async function allowClipboard(context: BrowserContext, browserName: string): Promise<void> {
    if (browserName === 'chromium') {
        await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    }
}

test('the sealed page shows the link with its key, the summary and the delete link', async ({
    page,
}) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    const { share, manage } = await create(page, {
        password: 'sealed',
        burn: true,
        items: [
            { type: 'plain', name: 'a.txt', text: 'first' },
            { type: 'plain', name: 'b.txt', text: 'second' },
        ],
    });
    await expect(page.locator('main h1')).toHaveText(en['create.done']!);

    const hexagon = page.locator('[data-hexagon="sealed"]');
    await expect(hexagon).toBeVisible();
    const closing = await ringAnimation(hexagon);
    expect(closing.name).not.toBe('none');
    expect(closing.iterations).toBe('1');
    expect(closing.duration).toBeGreaterThan(0);
    expect(closing.duration).toBeLessThanOrEqual(0.7);

    // The link as text, the key from `#` on highlighted in its own element.
    const link = page.locator('[data-share-link]');
    expect(await link.evaluate((element) => element.localName)).not.toBe('input');
    await expect(link.locator('input')).toHaveCount(0);
    expect(share).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/p\/[\w-]+#v1\./);
    const key = link.locator('[data-share-key]');
    await expect(key).toHaveText(share.slice(share.indexOf('#')));
    expect(await colour(key, 'color')).toBe(await token(page, '--cb-accent'));
    const note = page.getByText(en['create.keyNote']!, { exact: true });
    await expect(note).toBeVisible();
    await settled(page);
    const linkBox = (await link.boundingBox())!;
    expect((await note.boundingBox())!.y).toBeGreaterThanOrEqual(linkBox.y + linkBox.height);

    // One filled button: "Copy link" is the only accent background on the page.
    await page.mouse.move(0, 0);
    const copyLink = page.locator('[data-copy="share-link"]');
    await expect(copyLink).toHaveText(en['create.copyLink']!);
    expect(
        await elementsWith(
            page.locator('body'),
            'backgroundColor',
            await token(page, '--cb-accent'),
        ),
    ).toEqual(['button.cb-btn[data-copy]']);

    const summary = page.locator('[data-summary] li');
    await expect(summary).toHaveText([
        /^2 items · \d+ B$/,
        /^Expires in /,
        en['create.summary.password']!,
        en['create.summary.burn']!,
    ]);

    const row = page.locator('[data-delete-row]');
    expect(await row.evaluate((element) => getComputedStyle(element).borderTopStyle)).toBe(
        'dashed',
    );
    await expect(row.locator('[data-delete-link]')).toHaveText(manage);
    expect(manage).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/manage\/[\w-]+#/);
    await expect(row.locator('[data-copy="delete-link"]')).toHaveText(en['create.copy']!);
    await expect(row).toContainText(en['create.deleteLinkNote']!);
    const another = page.getByRole('link', { name: en['create.another']!, exact: true });
    await expect(another).toHaveAttribute('href', '/');

    // At 400 px the link wraps inside the column.
    await page.setViewportSize({ width: 400, height: 800 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        400,
    );
    const narrow = (await link.boundingBox())!;
    const column = (await page.locator('main').boundingBox())!;
    expect(narrow.x + narrow.width).toBeLessThanOrEqual(column.x + column.width + 0.5);
    expect(narrow.height).toBeGreaterThan(linkBox.height);
});

test('the summary says when there is no password and no expiry', async ({ page }) => {
    // The main instance keeps no capsule forever; this one does.
    await withInstance({ CAPSULEBIN__PASTES__ALLOW_PERMANENT: 'true' }, async (origin) => {
        await page.goto(`${origin}/`);
        await fillForm(page, { expiry: '', items: [{ type: 'plain', text: 'twelve bytes' }] });
        await page.click('[data-create-submit]');
        await sealedLinks(page);
        await expect(page.locator('[data-summary] li')).toHaveText([
            /^1 item · 12 B$/,
            en['expiry.none']!,
            en['create.summary.noPassword']!,
        ]);
    });
});

test('every copy button says "Copied" for two seconds', async ({ page, context, browserName }) => {
    await allowClipboard(context, browserName);
    const { share } = await create(page, { items: [{ type: 'plain', text: 'copy me' }] });
    const region = page.locator('[role="status"][aria-live="polite"]');

    const expectCopied = async (button: Locator, label: string) => {
        await button.click();
        // The button's own content waits hidden inside it: only what is shown counts.
        await expect(button).toHaveText(en['shell.copied']!, { useInnerText: true });
        await expect(button).toHaveClass(/\bcb-copied\b/);
        await expect(button.locator('svg[data-icon="check"]')).toBeVisible();
        await expect(button.page().locator('[role="status"][aria-live="polite"]')).toHaveText(
            en['shell.copiedToClipboard']!,
        );
        await expect(button).toHaveText(label, { timeout: 3_000, useInnerText: true });
        await expect(button).not.toHaveClass(/\bcb-copied\b/);
    };

    await expectCopied(page.locator('[data-copy="share-link"]'), en['create.copyLink']!);
    await expect(region).toHaveText('');
    await expectCopied(page.locator('[data-copy="delete-link"]'), en['create.copy']!);

    const view = await context.newPage();
    await view.goto(share);
    const copy = view.locator('[data-item-copy]');
    await expect(copy).toBeVisible({ timeout: 30_000 });
    await expectCopied(copy, en['view.copy']!);
});

test('opening and decrypting run the hexagon, then the content takes its place', async ({
    page,
}) => {
    const { share } = await create(page, { items: [{ type: 'plain', text: 'decrypted' }] });
    const view = await page.context().newPage();
    const describe = await hold(view, '**/api/v1/pastes/*/describe');
    const retrieve = await hold(view, '**/api/v1/pastes/*/retrieve');
    await view.goto(share);

    await describe.reached;
    await expectRunning(await expectState(view, 'running', en['view.opening']!));
    describe.release();
    await retrieve.reached;
    await expectRunning(await expectState(view, 'running', en['view.decrypting']!));
    retrieve.release();

    await expect(view.locator('.cb-item-panel')).toContainText('decrypted', { timeout: 30_000 });
    await expect(view.locator('[data-hexagon]')).toHaveCount(0);
});

test('the password prompt is locked and the key derivation runs the hexagon', async ({ page }) => {
    const password = 'locked away';
    const { share } = await create(page, {
        password,
        items: [{ type: 'plain', text: 'behind a password' }],
    });
    const view = await page.context().newPage();
    await view.goto(share);
    await expectState(view, 'locked', en['view.passwordTitle']!);

    const retrieve = await hold(view, '**/api/v1/pastes/*/retrieve');
    await view.fill('#pwd-input', password);
    await view.click('button[type=submit]');
    // The key derivation shows its progress with a running hexagon in the form.
    await expect(view.locator('.cb-kdf-progress [data-hexagon="running"]')).toBeAttached({
        timeout: 30_000,
    });
    await retrieve.reached;
    await expect(view.locator('main [data-hexagon="running"]')).toBeVisible();
    retrieve.release();
    await expect(view.locator('.cb-item-panel')).toContainText('behind a password', {
        timeout: 30_000,
    });
    await expect(view.locator('[data-hexagon]')).toHaveCount(0);
});

test('the burn warning is a warning, a burned or unknown capsule is empty', async ({ page }) => {
    const { share } = await create(page, {
        burn: true,
        items: [{ type: 'plain', text: 'once' }],
    });
    const first = await page.context().newPage();
    await first.goto(share);
    await expectWarning(first, await expectState(first, 'warning', en['view.burnWarning']!));
    await first.getByRole('button', { name: en['view.burnConfirm']!, exact: true }).click();
    await expect(first.locator('.cb-item-panel')).toContainText('once', { timeout: 30_000 });

    const second = await page.context().newPage();
    await second.goto(share);
    const gone = second.locator('main [data-hexagon="empty"]');
    await expect(gone).toBeVisible({ timeout: 30_000 });
    await expect(second.locator('main h1')).toHaveText(
        new RegExp(`^(${en['view.burned']}|${en['view.unavailable']})$`),
    );
    await expectEmpty(gone);

    const unknown = await page.context().newPage();
    await unknown.goto(`/p/AAAAAAAAAAAAAAAAAAAAAA${share.slice(share.indexOf('#'))}`);
    await expectEmpty(await expectState(unknown, 'empty', en['view.unavailable']!));
});

test('the management page runs, warns, runs and ends empty', async ({ page }) => {
    const { manage } = await create(page, { items: [{ type: 'plain', text: 'manage me' }] });
    const managePage = await page.context().newPage();
    const describe = await hold(managePage, '**/api/v1/pastes/*/describe');
    const remove = await hold(managePage, '**/api/v1/pastes/*', 'DELETE');
    await managePage.goto(manage);

    await describe.reached;
    await expectRunning(managePage.locator('main [data-hexagon="running"]'));
    describe.release();
    await expectWarning(
        managePage,
        await expectState(managePage, 'warning', en['manage.confirm']!),
    );

    await managePage.click('[data-delete-btn]');
    await remove.reached;
    await expectRunning(await expectState(managePage, 'running', en['manage.deleting']!));
    remove.release();
    await expectEmpty(await expectState(managePage, 'empty', en['manage.deleted']!));

    const again = await page.context().newPage();
    await again.goto(manage);
    await expectEmpty(await expectState(again, 'empty', en['manage.unavailable']!));
});

test('creating runs the hexagon, a failed create ends empty', async ({ browser }) => {
    // The failure is provoked on purpose: the engine logs the refused request and
    // the create page the error it handles. Only those lines are accepted.
    const context = await browser.newContext();
    const guard = await installGuard(context);
    const page = await context.newPage();
    await fill(page, { items: [{ type: 'plain', text: 'never stored' }] });
    const createRequest = await hold(page, '**/api/v1/pastes', 'POST', (route) =>
        route.fulfill({
            status: 503,
            contentType: 'application/problem+json',
            body: JSON.stringify({
                title: 'Service Unavailable',
                status: 503,
                code: 'unavailable',
            }),
        }),
    );
    await page.click('[data-create-submit]');

    await createRequest.reached;
    await expectRunning(await expectState(page, 'running', en['create.encrypting']!));
    createRequest.release();
    await expectEmpty(await expectState(page, 'empty', en['create.error']!));
    await context.close();
    expect(
        guard.problems.filter(
            (problem) => !/^console error: ((Api)?Error\b|.*\b503\b)/.test(problem),
        ),
    ).toEqual([]);
});

test.describe('with reduced motion', () => {
    test.use({ reducedMotion: 'reduce' });

    test('the hexagons stand still', async ({ page }) => {
        const { share } = await create(page, { items: [{ type: 'plain', text: 'still' }] });
        const sealed = await ringAnimation(page.locator('[data-hexagon="sealed"]'));
        expect(sealed.name === 'none' || sealed.duration === 0).toBe(true);

        const view = await page.context().newPage();
        const describe = await hold(view, '**/api/v1/pastes/*/describe');
        await view.goto(share);
        await describe.reached;
        const running = view.locator('main [data-hexagon="running"]');
        await expect(running).toBeVisible();
        const static_ = await ringAnimation(running);
        expect(static_.name === 'none' || static_.duration === 0).toBe(true);
        expect(static_.dashes).toBe('none');
        describe.release();
        await expect(view.locator('.cb-item-panel')).toContainText('still', { timeout: 30_000 });
    });
});
