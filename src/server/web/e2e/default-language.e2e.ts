import { type ChildProcess, spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Browser, BrowserContext, Page } from '@playwright/test';
import { create } from './create';
import { type Guard, expect, installGuard, test as base } from './guard';

/**
 * Acceptance tests for an instance whose default language is German
 * (`[i18n] default_language = "de"`), in a real engine against the release
 * binary with the shells of the web build (AC2, AC3, AC6, AC14, AC15, AC28 for
 * D = de).
 *
 * Every other browser test runs against the instance of playwright.config.ts,
 * whose default is `en`. Here the roles are swapped: German pages have no path
 * prefix, English ones live under `/en/`, and `/de/…` is redirected away. The
 * server fills the default language and the link prefix into the generated
 * shells, and `language-init.js` and the language switch must follow that
 * configured default rather than a built-in `en`.
 *
 * The instance is started once per worker on a free port with its own database.
 */

const BINARY = resolve(
    fileURLToPath(new URL('..', import.meta.url)),
    process.env.CAPSULEBIN_BINARY ?? '../../../target/release/capsulebin',
);

function freePort(): Promise<number> {
    return new Promise((done, failed) => {
        const probe = createServer();
        probe.on('error', failed);
        probe.listen(0, '127.0.0.1', () => {
            const address = probe.address();
            const port = typeof address === 'object' && address ? address.port : 0;
            probe.close(() => done(port));
        });
    });
}

async function ready(origin: string, server: ChildProcess): Promise<void> {
    for (let attempt = 0; attempt < 200; attempt++) {
        if (server.exitCode !== null) throw new Error(`the server exited with ${server.exitCode}`);
        try {
            if ((await fetch(`${origin}/health/ready`)).ok) return;
        } catch {
            /* Not listening yet. */
        }
        await new Promise((wait) => setTimeout(wait, 100));
    }
    throw new Error(`${origin} did not become ready`);
}

const test = base.extend<object, { origin: string }>({
    origin: [
        async ({}, use) => {
            const port = await freePort();
            const origin = `http://127.0.0.1:${port}`;
            const data = mkdtempSync(join(tmpdir(), 'capsulebin-e2e-de-'));
            const server = spawn(BINARY, ['serve'], {
                stdio: ['ignore', 'ignore', 'inherit'],
                env: {
                    ...process.env,
                    CAPSULEBIN__SERVER__BIND: `127.0.0.1:${port}`,
                    CAPSULEBIN__SERVER__PUBLIC_URL: origin,
                    CAPSULEBIN__DATABASE__PATH: join(data, 'capsulebin.db'),
                    CAPSULEBIN__CREATION__MODE: 'public',
                    CAPSULEBIN__I18N__DEFAULT_LANGUAGE: 'de',
                    CAPSULEBIN__LEGAL__OPERATOR__TEXT: 'Example Operator GmbH',
                    CAPSULEBIN_LOG: 'warn',
                },
            });
            try {
                await ready(origin, server);
                await use(origin);
            } finally {
                server.kill();
            }
        },
        { scope: 'worker' },
    ],
});

/** A browser with its own locale on the German-default instance, under the guard. */
async function browserIn(
    browser: Browser,
    origin: string,
    locale: string,
): Promise<{ context: BrowserContext; guard: Guard }> {
    const context = await browser.newContext({ locale, baseURL: origin });
    return { context, guard: await installGuard(context) };
}

/** Every URL the main frame of `page` shows, including the ones it only passes through. */
function recordUrls(page: Page): string[] {
    const urls: string[] = [];
    page.on('framenavigated', (frame) => {
        if (frame === page.mainFrame()) urls.push(frame.url());
    });
    return urls;
}

/** The `href` of every link to a local path, except the API, assets and scripts. */
async function internalLinks(page: Page): Promise<string[]> {
    const hrefs = await page
        .locator('a[href^="/"]')
        .evaluateAll((links) => links.map((link) => link.getAttribute('href') ?? ''));
    return hrefs.filter(
        (href) => !href.startsWith('/api/') && !href.startsWith('/assets/') && !/\.js$/.test(href),
    );
}

function parts(link: string): { id: string; fragment: string } {
    const url = new URL(link);
    return { id: url.pathname.split('/').at(-1) ?? '', fragment: url.hash };
}

/**
 * Creates a capsule on the German-default instance in a German browser and
 * returns its share link. The create page is `/`, which `create()` opens for
 * `en` - the language without a prefix on the instance it was written for.
 */
async function createInGerman(browser: Browser, origin: string, text: string): Promise<string> {
    const { context, guard } = await browserIn(browser, origin, 'de-DE');
    const page = await context.newPage();
    const { share } = await create(page, { items: [{ type: 'plain', text }] });
    await context.close();
    expect(guard.problems).toEqual([]);
    return share;
}

const SOME_ID = '2pS3xQ7Rk9mNpLz4AbCdEA';

test('the server serves German without a prefix and English under /en/', async ({
    origin,
    request,
}) => {
    const languagesMeta =
        /<meta name="capsulebin-languages" content="en de" data-default="de"\s*\/?>/;

    // AC2, AC6: every page in both languages, with the configured default in the meta.
    for (const [prefix, language] of [
        ['', 'de'],
        ['/en', 'en'],
    ] as const) {
        for (const path of ['/', `/p/${SOME_ID}`, `/manage/${SOME_ID}`, '/report', '/imprint']) {
            const answer = await request.get(`${origin}${prefix}${path}`, { maxRedirects: 0 });
            expect(answer.status(), `${prefix}${path}`).toBe(200);
            expect(answer.headers()['cache-control'], `${prefix}${path}`).toBe('no-store');
            const html = await answer.text();
            expect(html, `${prefix}${path}`).toContain(`<html lang="${language}">`);
            expect(html, `${prefix}${path}`).toMatch(languagesMeta);
            expect(html.indexOf('capsulebin-languages'), `${prefix}${path}`).toBeLessThan(
                html.indexOf('/language-init.js'),
            );
            expect(html, `${prefix}${path}`).not.toContain('%%capsulebin:');
            expect(html, `${prefix}${path}`).not.toContain('<!--capsulebin:');
            // Internal links carry the shell's prefix: none in German, /en in English.
            expect(html, `${prefix}${path}`).toContain(`href="${prefix}/"`);
            expect(html, `${prefix}${path}`).toContain(`href="${prefix}/imprint"`);
            expect(html, `${prefix}${path}`).not.toContain('href="/de/');
            if (language === 'de') expect(html, path).not.toContain('href="/en/');
        }
    }

    // AC3: the prefix of the default language is redirected away, whatever the browser asks for.
    for (const [path, location] of [
        ['/de', '/'],
        ['/de/', '/'],
        [`/de/p/${SOME_ID}`, `/p/${SOME_ID}`],
        [`/de/manage/${SOME_ID}`, `/manage/${SOME_ID}`],
        [`/de/report?id=${SOME_ID}`, `/report?id=${SOME_ID}`],
        ['/de/imprint', '/imprint'],
    ] as const) {
        for (const acceptLanguage of ['en-US,en;q=0.9', 'de-DE']) {
            const answer = await request.get(`${origin}${path}`, {
                maxRedirects: 0,
                headers: { 'accept-language': acceptLanguage },
            });
            expect(answer.status(), path).toBe(302);
            expect(answer.headers()['location'], path).toBe(location);
            expect(answer.headers()['cache-control'], path).toBe('no-store');
            expect(answer.headers()['vary'] ?? '', path).not.toMatch(/accept-language/i);
            expect(answer.headers()['content-security-policy'], path).toBeTruthy();
        }
    }

    // English is not the default here, so its prefix is a page, not a redirect.
    const english = await request.get(`${origin}/en/`, {
        maxRedirects: 0,
        headers: { 'accept-language': 'de-DE' },
    });
    expect(english.status()).toBe(200);
    expect(english.headers()['vary'] ?? '').not.toMatch(/accept-language/i);
});

test('a German browser stays on the prefix-less pages', async ({ origin, browser }) => {
    const { context, guard } = await browserIn(browser, origin, 'de-DE');
    const page = await context.newPage();
    const urls = recordUrls(page);

    await page.goto('/');
    await expect(page.locator('html')).toHaveAttribute('lang', 'de');
    await expect(page.locator('body')).toContainText('Verschlüsselt teilen, ohne Konto');
    const { share, manage } = await create(page, {
        items: [{ type: 'plain', text: 'Ohne Präfix auf Deutsch' }],
    });
    const { id, fragment } = parts(share);
    // AC15: the links carry no prefix, whatever the default language is.
    expect(share).toBe(`${origin}/p/${id}${fragment}`);
    expect(fragment).toMatch(/^#v1\./);
    expect(manage).toMatch(new RegExp(`^${origin}/manage/${id}#v1\\.`));
    expect(urls.filter((url) => /\/(en|de)(\/|$)/.test(new URL(url).pathname))).toEqual([]);

    const view = await context.newPage();
    const viewUrls = recordUrls(view);
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toContainText('Ohne Präfix auf Deutsch', {
        timeout: 30_000,
    });
    await expect(view.locator('html')).toHaveAttribute('lang', 'de');
    expect(new URL(view.url()).pathname).toBe(`/p/${id}`);
    expect(viewUrls.filter((url) => new URL(url).pathname !== `/p/${id}`)).toEqual([]);

    // AC14: internal links of a page in the default language have no prefix.
    const links = await internalLinks(view);
    expect(links).toContain(`/report?id=${id}`);
    expect(links.filter((href) => /^\/(en|de)(\/|$)/.test(href))).toEqual([]);

    const managePage = await context.newPage();
    await managePage.goto(manage);
    await expect(managePage.locator('[data-delete-btn]')).toBeVisible({ timeout: 30_000 });
    expect(new URL(managePage.url()).pathname).toBe(`/manage/${id}`);
    await expect(managePage.locator('html')).toHaveAttribute('lang', 'de');

    await context.close();
    expect(guard.problems).toEqual([]);
});

test('an English browser is moved to /en/ with the key and decrypts', async ({
    origin,
    browser,
}) => {
    const share = await createInGerman(browser, origin, 'Für englische Leser');
    const { id, fragment } = parts(share);

    const { context, guard } = await browserIn(browser, origin, 'en-US');

    // AC28: the prefix-less share link is German here; the browser language moves it to /en/.
    const view = await context.newPage();
    const urls = recordUrls(view);
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toContainText('Für englische Leser', {
        timeout: 30_000,
    });
    expect(urls).toContain(`${origin}/en/p/${id}${fragment}`);
    expect(new URL(view.url()).pathname).toBe(`/en/p/${id}`);
    await expect(view.locator('html')).toHaveAttribute('lang', 'en');

    // AC14: every internal link of the English page is under /en/.
    const links = await internalLinks(view);
    expect(links).toContain(`/en/report?id=${id}`);
    expect(links.filter((href) => !href.startsWith('/en/'))).toEqual([]);

    // AC3 + AC28: a /de/ link is redirected by the server to the prefix-less path and moved on
    // by the head script; the key survives both steps.
    const viaGerman = await context.newPage();
    const viaGermanUrls = recordUrls(viaGerman);
    await viaGerman.goto(`${origin}/de/p/${id}${fragment}`);
    await expect(viaGerman.locator('.cb-item-panel')).toContainText('Für englische Leser', {
        timeout: 30_000,
    });
    expect(viaGermanUrls).toContain(`${origin}/en/p/${id}${fragment}`);
    expect(new URL(viaGerman.url()).pathname).toBe(`/en/p/${id}`);

    // The stored choice outranks the browser language: /en/ now leads to the prefix-less page.
    await view.evaluate(() => localStorage.setItem('capsulebin.language', 'de'));
    const chosen = await context.newPage();
    const chosenUrls = recordUrls(chosen);
    await chosen.goto(`${origin}/en/p/${id}${fragment}`);
    await expect(chosen.locator('.cb-item-panel')).toContainText('Für englische Leser', {
        timeout: 30_000,
    });
    expect(chosenUrls).toContain(`${origin}/p/${id}${fragment}`);
    expect(new URL(chosen.url()).pathname).toBe(`/p/${id}`);
    await expect(chosen.locator('html')).toHaveAttribute('lang', 'de');

    await context.close();
    expect(guard.problems).toEqual([]);
});

test('the language switch leads to /en/ for English and to the prefix-less page for German', async ({
    origin,
    browser,
}) => {
    const { context, guard } = await browserIn(browser, origin, 'de-DE');
    const page = await context.newPage();
    await page.goto('/report?id=' + SOME_ID);
    const switcher = page.locator('[data-language-switch]');
    await expect(switcher).toBeVisible();
    await expect(switcher.getByRole('button', { name: 'Deutsch' })).toHaveAttribute(
        'aria-current',
        'true',
    );

    await switcher.getByRole('button', { name: 'English' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');
    expect(new URL(page.url()).pathname).toBe('/en/report');
    expect(new URL(page.url()).search).toBe(`?id=${SOME_ID}`);
    expect(await page.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('en');
    await expect(switcher.getByRole('button', { name: 'English' })).toHaveAttribute(
        'aria-current',
        'true',
    );

    // The stored choice holds on the next prefix-less page, against the browser language.
    await page.goto('/');
    await expect(page.locator('html')).toHaveAttribute('lang', 'en');
    expect(new URL(page.url()).pathname).toBe('/en/');

    await switcher.getByRole('button', { name: 'Deutsch' }).click();
    await expect(page.locator('html')).toHaveAttribute('lang', 'de');
    expect(new URL(page.url()).pathname).toBe('/');
    expect(await page.evaluate(() => localStorage.getItem('capsulebin.language'))).toBe('de');

    await context.close();
    expect(guard.problems).toEqual([]);
});
