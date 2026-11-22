import type { Locator, Page } from '@playwright/test';
import { create, fill, openCreate, sealedLinks } from './create';
import { expect, test } from './guard';
import { colour, contrast, expectNoHorizontalScroll, px, settled, token } from './style';

/**
 * One column and depth in a real engine: header, content and footer start and
 * end at the same x on every page, with 16 px gutters on a phone; the centred
 * state pages stay centred; the surfaces (view sheet, editor frame, link panel)
 * cast a shadow, also under a theme chosen with the toggle against the system
 * preference, and the light page background is a tint the white surface stands
 * out from.
 */

const WIDE = { width: 1280, height: 800 };
const NARROW = { width: 400, height: 800 };
const GUTTER = 16;

const THREE_ITEMS = [
    { type: 'plain', name: 'first.txt', text: 'the first item' },
    { type: 'markdown', name: 'second.md', text: '# Second\n\nThe second item.' },
    { type: 'code', name: 'third.rs', lang: 'rust', text: 'fn main() {}' },
] as const;

interface Edges {
    left: number;
    right: number;
}

/** Left edge of the first and right edge of the last rendered child of the region. */
async function childEdges(region: Locator): Promise<Edges> {
    await settled(region.page());
    return region.evaluate((region) => {
        const boxes = [...region.children]
            .map((child) => child.getBoundingClientRect())
            .filter((box) => box.width > 0 && box.height > 0);
        return {
            left: Math.min(...boxes.map((box) => box.left)),
            right: Math.max(...boxes.map((box) => box.right)),
        };
    });
}

/** The element's content box: its box without the padding. */
async function contentEdges(locator: Locator): Promise<Edges> {
    await settled(locator.page());
    return locator.evaluate((element) => {
        const box = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return {
            left: box.left + Number.parseFloat(style.paddingLeft),
            right: box.right - Number.parseFloat(style.paddingRight),
        };
    });
}

async function borderEdges(locator: Locator): Promise<Edges> {
    await settled(locator.page());
    return locator.evaluate((element) => {
        const box = element.getBoundingClientRect();
        return { left: box.left, right: box.right };
    });
}

/** The layout viewport's width: without a classic scrollbar, where the engine has one. */
const pageWidth = (page: Page) => page.evaluate(() => document.documentElement.clientWidth);

/**
 * Header, content and footer of the page share one column: equal left and right
 * edges (± 1 px). `surfaces` are the page's own full-width elements, which use
 * that column too.
 */
async function expectOneColumn(page: Page, surfaces: string[], where: string): Promise<void> {
    const head = await childEdges(page.locator('header.cb-head'));
    const main = await contentEdges(page.locator('main.cb-main'));
    const regions: Record<string, Edges> = {
        content: main,
        footer: await childEdges(page.locator('footer.cb-foot')),
    };
    for (const selector of surfaces) {
        regions[selector] = await borderEdges(page.locator(selector));
    }
    for (const [name, edges] of Object.entries(regions)) {
        expect(Math.abs(edges.left - head.left), `${where}: left edge of ${name}`).toBeLessThan(
            1.01,
        );
        expect(Math.abs(edges.right - head.right), `${where}: right edge of ${name}`).toBeLessThan(
            1.01,
        );
    }
}

/** At a phone width: 16 px gutters around header, content and footer, nothing scrolls sideways. */
async function expectGutters(page: Page, surfaces: string[], where: string): Promise<void> {
    await expectNoHorizontalScroll(page);
    const end = (await pageWidth(page)) - GUTTER;
    const regions: Record<string, Edges> = {
        header: await childEdges(page.locator('header.cb-head')),
        content: await contentEdges(page.locator('main.cb-main')),
    };
    for (const selector of surfaces) {
        regions[selector] = await borderEdges(page.locator(selector));
    }
    for (const [name, edges] of Object.entries(regions)) {
        expect(Math.abs(edges.left - GUTTER), `${where}: left gutter of ${name}`).toBeLessThan(
            1.01,
        );
        expect(Math.abs(edges.right - end), `${where}: right gutter of ${name}`).toBeLessThan(1.01);
    }
    // The footer wraps at this width: it starts at the gutter and stays within the column.
    const footer = await childEdges(page.locator('footer.cb-foot'));
    expect(Math.abs(footer.left - GUTTER), `${where}: left gutter of footer`).toBeLessThan(1.01);
    expect(footer.right, `${where}: footer within the column`).toBeLessThanOrEqual(end + 1);
}

/** The view page of `share` in the same tab, once the content is shown. */
async function openView(page: Page, share: string): Promise<void> {
    await page.goto(share);
    await expect(page.locator('.cb-item-panel')).toBeVisible({ timeout: 30_000 });
}

test('header, content and footer share one column on every page', async ({ page }) => {
    test.slow();
    await page.setViewportSize(WIDE);
    const one = await create(page, { items: [{ type: 'plain', text: 'one column' }] });
    const three = await create(page, { items: [...THREE_ITEMS] });

    // Each page and its own full-width surfaces. The sealed page is the last one
    // created, still shown by this tab.
    const pages: [name: string, open: () => Promise<void>, surfaces: string[]][] = [
        ['create', () => openCreate(page), ['[data-create-form]', '[data-editor]']],
        ['view with one item', () => openView(page, one.share), ['[data-sheet]']],
        ['view with three items', () => openView(page, three.share), ['[data-sheet]']],
        [
            'report',
            async () => {
                await page.goto('/report');
                await expect(page.locator('[data-report-form]')).toBeVisible();
            },
            [],
        ],
        [
            'imprint',
            async () => {
                await page.goto('/imprint');
                await expect(page.locator('main h1')).toBeVisible();
            },
            [],
        ],
    ];

    for (const colorScheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme });
        for (const viewport of [WIDE, NARROW]) {
            await page.setViewportSize(viewport);
            const check = viewport === WIDE ? expectOneColumn : expectGutters;

            await fill(page, { items: [{ type: 'plain', text: 'sealed in the column' }] });
            await page.click('[data-create-submit]');
            await sealedLinks(page);
            await check(
                page,
                ['.cb-sealed', '.cb-link-panel', '[data-delete-row]'],
                `sealed ${colorScheme} ${viewport.width}`,
            );

            for (const [name, open, surfaces] of pages) {
                await open();
                await check(page, surfaces, `${name} ${colorScheme} ${viewport.width}`);
            }
        }
    }
});

test('state pages keep their centred layout', async ({ page }) => {
    await page.setViewportSize(WIDE);
    const { share, manage } = await create(page, { items: [{ type: 'plain', text: 'centred' }] });

    const states = {
        'unknown capsule': `/p/AAAAAAAAAAAAAAAAAAAAAA${share.slice(share.indexOf('#'))}`,
        'delete confirmation': manage,
    };
    for (const [name, path] of Object.entries(states)) {
        await page.goto(path);
        const hexagon = page.locator('main [data-hexagon]');
        await expect(hexagon, name).toBeVisible({ timeout: 30_000 });
        await expect(page.locator('main h1'), name).toBeVisible();
        const middle = (await pageWidth(page)) / 2;
        const column = await childEdges(page.locator('header.cb-head'));
        for (const element of [hexagon, page.locator('main h1')]) {
            const edges = await borderEdges(element);
            expect(Math.abs((edges.left + edges.right) / 2 - middle), name).toBeLessThan(1.01);
            // Narrower than the column the header spans, not stretched to it.
            expect(edges.left, name).toBeGreaterThan(column.left + 1);
            expect(edges.right, name).toBeLessThan(column.right - 1);
        }
    }
});

/** The theme of the system and, if any, the one chosen with the toggle against it. */
const THEMES = [
    { system: 'light', chosen: null },
    { system: 'dark', chosen: null },
    { system: 'light', chosen: 'dark' },
    { system: 'dark', chosen: 'light' },
] as const;

for (const { system, chosen } of THEMES) {
    const theme = chosen ?? system;
    const how = chosen ? `the ${chosen} theme chosen on a ${system} system` : `the ${system} theme`;

    test(`surfaces cast a shadow in ${how}, the light background a tint behind them`, async ({
        page,
    }) => {
        await page.emulateMedia({ colorScheme: system });
        await page.setViewportSize(WIDE);
        const { share } = await create(page, { items: [...THREE_ITEMS] });
        const shadowOf = (selector: string) =>
            page.locator(selector).evaluate((element) => getComputedStyle(element).boxShadow);

        if (chosen) {
            // The stored choice of the theme toggle, applied by the head script.
            await page.evaluate(
                (chosen) => localStorage.setItem('capsulebin.theme', chosen),
                chosen,
            );
        }
        await openCreate(page);
        if (chosen) await expect(page.locator('html')).toHaveAttribute('data-theme', chosen);
        // Outside the frame, whose focus ring is a shadow of its own.
        await page.locator('#create-title').focus();
        expect(await shadowOf('[data-editor]'), 'editor').not.toBe('none');

        const background = await colour(page.locator('body'), 'backgroundColor');
        const surface = await colour(page.locator('[data-editor]'), 'backgroundColor');
        expect(background).toBe(await token(page, '--cb-bg'));
        expect(surface).toBe(await token(page, '--cb-surface'));
        if (theme === 'light') {
            expect(contrast(background, surface)).toBeGreaterThanOrEqual(1.06);
            expect(background).not.toBe('#fbfcfc');
        }

        await page.fill('[data-item-text]', 'depth');
        await page.click('[data-create-submit]');
        await sealedLinks(page);
        expect(await shadowOf('.cb-link-panel'), 'link panel').not.toBe('none');

        await openView(page, share);
        expect(await shadowOf('[data-sheet]'), 'sheet').not.toBe('none');
        // A real offset or blur, not a zero-sized shadow.
        const lengths = (await shadowOf('[data-sheet]')).match(/-?[\d.]+px/g) ?? [];
        expect(lengths.some((length) => px(length) !== 0)).toBe(true);
    });
}
