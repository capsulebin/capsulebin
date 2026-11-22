import { MAX_ITEM_BYTES } from '@capsulebin/core';
import type { Locator, Page } from '@playwright/test';
import { create, fill, prefixOf, sealedLinks } from './create';
import { catalogue } from './german';
import { expect, test } from './guard';
import { colour, elementsWith, px, settled, token, transitionOf } from './style';

/**
 * The page design in a real engine: quiet header and footer, the radius scale,
 * transitions and reduced motion, the narrow column at 400 px, the accent kept
 * for the active tab and the one primary action, the reading measure, and the
 * create page's editor-first layout. Measured from computed styles and boxes,
 * which jsdom does not have.
 */

const en = catalogue('en');

const NARROW = { width: 400, height: 800 };
const WIDE = { width: 1280, height: 800 };

/** Top, right, bottom and left border width of the element, in px. */
async function borderWidths(locator: Locator): Promise<number[]> {
    return locator.evaluate((element) => {
        const style = getComputedStyle(element);
        return [
            style.borderTopWidth,
            style.borderRightWidth,
            style.borderBottomWidth,
            style.borderLeftWidth,
        ].map((width) => Number.parseFloat(width));
    });
}

/** The four corner radii of the element, in px. */
async function radii(locator: Locator): Promise<number[]> {
    return locator.evaluate((element) => {
        const style = getComputedStyle(element);
        return [
            style.borderTopLeftRadius,
            style.borderTopRightRadius,
            style.borderBottomRightRadius,
            style.borderBottomLeftRadius,
        ].map((radius) => Number.parseFloat(radius));
    });
}

async function expectNoBorder(locator: Locator): Promise<void> {
    for (const element of await locator.all())
        expect(await borderWidths(element)).toEqual([0, 0, 0, 0]);
}

/** The element's box once the page's entry animation has ended. */
async function box(
    locator: Locator,
): Promise<{ x: number; y: number; width: number; height: number }> {
    await settled(locator.page());
    const found = await locator.boundingBox();
    expect(found, 'the element has a box').not.toBeNull();
    return found!;
}

/** `inner` lies within `outer`'s box (half a pixel of rounding allowed). */
async function expectWithin(inner: Locator, outer: Locator): Promise<void> {
    const a = await box(inner);
    const b = await box(outer);
    expect(a.x).toBeGreaterThanOrEqual(b.x - 0.5);
    expect(a.x + a.width).toBeLessThanOrEqual(b.x + b.width + 0.5);
}

async function expectNoHorizontalScroll(page: Page): Promise<void> {
    const width = page.viewportSize()!.width;
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
    );
}

/** The view page of `share` in a new page of the context, once the content is shown. */
async function openView(page: Page, share: string): Promise<Page> {
    const view = await page.context().newPage();
    await view.setViewportSize(page.viewportSize() ?? WIDE);
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toBeVisible({ timeout: 30_000 });
    return view;
}

const THREE_ITEMS = [
    { type: 'plain', name: 'first.txt', text: 'the first item' },
    { type: 'markdown', name: 'second.md', text: '# Second\n\nThe second item.' },
    { type: 'code', name: 'third.rs', lang: 'rust', text: 'fn main() {}' },
] as const;

for (const [language, locale] of [
    ['en', 'en-US'],
    ['de', 'de-DE'],
] as const) {
    test.describe(`pages in ${language}`, () => {
        test.use({ locale });

        test('header and footer have no band, no border and no accent fill', async ({ page }) => {
            test.slow();
            const { share, manage } = await create(
                page,
                { items: [{ type: 'plain', text: 'chrome' }] },
                language,
            );
            const prefix = prefixOf(language);
            const pages = {
                create: `${prefix}/`,
                view: share,
                manage,
                report: `${prefix}/report`,
                imprint: `${prefix}/imprint`,
            };

            for (const colorScheme of ['light', 'dark'] as const) {
                await page.emulateMedia({ colorScheme });
                for (const [name, path] of Object.entries(pages)) {
                    await page.goto(path);
                    const current = page.locator('[data-language-switch] [aria-current="true"]');
                    await expect(current, `${name} ${colorScheme}`).toBeVisible();
                    await expect(page.locator('html')).toHaveAttribute('lang', language);

                    const body = await colour(page.locator('body'), 'backgroundColor');
                    const accent = await token(page, '--cb-accent');
                    for (const selector of ['header.cb-head', 'footer.cb-foot']) {
                        const region = page.locator(selector);
                        expect(
                            [body, 'rgba(0, 0, 0, 0)'],
                            `${selector} on ${name} ${colorScheme}`,
                        ).toContain(await colour(region, 'backgroundColor'));
                        expect(await borderWidths(region)).toEqual([0, 0, 0, 0]);
                    }
                    expect(
                        await elementsWith(
                            page.locator('header.cb-head'),
                            'backgroundColor',
                            accent,
                        ),
                        `accent fill in the header of ${name} ${colorScheme}`,
                    ).toEqual([]);

                    // The current language as a quiet text button.
                    expect(await current.evaluate((element) => element.localName)).toBe('button');
                    await expect(current).toHaveAttribute('lang', language);
                    await expect(current).not.toHaveText('');
                }
            }
        });
    });
}

test('controls have the control radius, surfaces the surface radius', async ({ page }) => {
    await page.setViewportSize(WIDE);
    await fill(page, { items: [...THREE_ITEMS] });

    for (const selector of [
        '[data-create-submit]',
        '#create-password',
        '#create-item-name',
        '#create-expiry',
        '#create-item-type',
    ]) {
        expect(await radii(page.locator(selector)), selector).toEqual([8, 8, 8, 8]);
    }
    expect(await radii(page.locator('[data-editor]'))).toEqual([12, 12, 12, 12]);

    // Tab bar and item action bar: borderless buttons.
    await expectNoBorder(page.locator('[data-item-tabs] button'));
    await expectNoBorder(page.locator('[data-tabs-row] > .cb-create-menu > button'));
    await expectNoBorder(page.locator('[data-item-actions] button'));

    await page.click('[data-create-submit]');
    const { share } = await sealedLinks(page);
    expect(await radii(page.locator('.cb-link-panel'))).toEqual([12, 12, 12, 12]);

    const view = await openView(page, share);
    expect(await radii(view.locator('[data-sheet]'))).toEqual([12, 12, 12, 12]);
    await view.locator('#cb-item-tab-1').click();
    await expect(view.locator('.cb-view-switch')).toBeVisible();
    await expectNoBorder(view.locator('[data-sheet] [role="tablist"] button'));
    await expectNoBorder(view.locator('[data-items-all]'));
    await expectNoBorder(view.locator('[data-sheet] .cb-bar-actions button'));
});

test('buttons and tabs change colour in 120 ms, and nothing moves with reduced motion', async ({
    page,
}) => {
    await fill(page, { items: [...THREE_ITEMS] });
    const controls = {
        button: page.locator('[data-create-submit]'),
        quiet: page.locator('[data-item-add]'),
        tab: page.locator('[data-item-tabs] .cb-itemtab').first(),
    };
    for (const [name, control] of Object.entries(controls)) {
        expect(await transitionOf(control, 'background-color'), name).toBe(0.12);
        expect(await transitionOf(control, 'color'), name).toBe(0.12);
    }
    // The entry animation as it plays without reduced motion.
    const entry = () =>
        page.locator('main').evaluate((main) => {
            main.classList.add('cb-enter');
            const { animationName, animationDuration } = getComputedStyle(main);
            main.classList.remove('cb-enter');
            return { animationName, duration: Number.parseFloat(animationDuration) };
        });
    expect((await entry()).duration).toBeGreaterThan(0);

    await page.emulateMedia({ reducedMotion: 'reduce' });
    for (const [name, control] of Object.entries(controls)) {
        for (const property of ['background-color', 'color']) {
            expect(await transitionOf(control, property), `${name} ${property}`).toBeFalsy();
        }
    }
    const still = await entry();
    expect(still.animationName === 'none' || still.duration === 0).toBe(true);
});

test('at 400 px nothing overflows and item actions show their icon only', async ({ page }) => {
    await page.setViewportSize(NARROW);
    await fill(page, { items: [...THREE_ITEMS] });

    await expectNoHorizontalScroll(page);
    const editorActions = page.locator('[data-item-actions] button:visible');
    expect(await editorActions.count()).toBeGreaterThan(0);
    for (const button of await editorActions.all()) {
        await expectWithin(button, page.locator('[data-editor]'));
    }
    await expect(
        page.getByRole('button', { name: en['create.remove']!, exact: true }),
    ).toBeVisible();
    for (const label of await page.locator('[data-item-actions] .cb-bar-label').all()) {
        expect((await box(label)).width).toBeLessThanOrEqual(1);
    }

    await page.click('[data-create-submit]');
    const { share } = await sealedLinks(page);
    const view = await openView(page, share);
    // Whether the tab fits at this width is up to the engine's fonts; "All N" reaches it either way.
    const tab = view.locator('#cb-item-tab-1');
    if (await tab.isVisible()) {
        await tab.click();
    } else {
        await view.locator('[data-items-all]').click();
        await view.getByRole('menuitemradio', { name: /second\.md/ }).click();
    }
    await expect(view.locator('.cb-view-switch')).toBeVisible();

    await expectNoHorizontalScroll(view);
    const sheet = view.locator('[data-sheet]');
    const actions = view.locator('[data-sheet] .cb-bar-actions button');
    for (const button of await actions.all()) await expectWithin(button, sheet);
    for (const name of [en['view.copy']!, en['view.download']!]) {
        await expect(view.getByRole('button', { name, exact: true })).toBeVisible();
    }
    for (const label of await view.locator('[data-sheet] .cb-bar-actions .cb-bar-label').all()) {
        expect((await box(label)).width).toBeLessThanOrEqual(1);
    }

    // At 1280 px the labels are back.
    await view.setViewportSize(WIDE);
    for (const label of await view.locator('[data-sheet] .cb-bar-actions .cb-bar-label').all()) {
        expect((await box(label)).width).toBeGreaterThan(1);
    }
    await expect(view.locator('[data-sheet] .cb-bar-actions')).toContainText(en['view.copy']!);
});

test('in a Markdown capsule only the active tab is in the accent', async ({ page }) => {
    await page.setViewportSize(WIDE);
    const { share } = await create(page, {
        items: [
            { type: 'markdown', name: 'one.md', text: '# One\n\nPlain words, **strong** words.' },
            { type: 'markdown', name: 'two.md', text: '## Two\n\n- a list\n- of words' },
        ],
    });
    const view = await openView(page, share);
    // The mouse rests outside the page content.
    await view.mouse.move(0, 0);
    const accent = await token(view, '--cb-accent');

    const pressed = view.locator('.cb-view-switch [aria-pressed="true"]');
    await expect(pressed).toHaveText(en['view.preview']!);
    expect(await colour(pressed, 'backgroundColor')).not.toBe(accent);
    expect(await colour(pressed, 'color')).not.toBe(accent);

    const main = view.locator('main');
    expect(await elementsWith(main, 'backgroundColor', accent)).toEqual([]);
    const active = '.cb-itemtab[aria-selected="true"]';
    expect(await elementsWith(main, 'color', accent, active)).toEqual([]);
    // The active tab is the one accent indicator.
    expect(await colour(view.locator(`${active} .cb-itemtab-kind`), 'color')).toBe(accent);

    await view.locator('.cb-view-switch button').nth(1).click();
    await expect(view.locator('.cb-view-switch [aria-pressed="true"]')).toHaveText(
        en['view.source']!,
    );
    expect(await elementsWith(main, 'backgroundColor', accent)).toEqual([]);
    expect(await elementsWith(main, 'color', accent, active)).toEqual([]);
});

test('rendered Markdown keeps the reading measure', async ({ page }) => {
    await page.setViewportSize(WIDE);
    const paragraph = 'A line of prose that runs on and on, well past any reading measure. '.repeat(
        12,
    );
    const { share } = await create(page, {
        items: [{ type: 'markdown', name: 'long.md', text: `# Measure\n\n${paragraph}` }],
    });
    const view = await openView(page, share);
    const prose = view.locator('.cb-item-panel .cb-prose');
    const measured = await prose.evaluate((element) => {
        const style = getComputedStyle(element);
        const measure = getComputedStyle(document.documentElement)
            .getPropertyValue('--cb-measure')
            .trim();
        const context = document.createElement('canvas').getContext('2d')!;
        context.font = style.font;
        const ch = context.measureText('0').width;
        const sheet = element.closest('[data-sheet]')!;
        return {
            maxWidth: style.maxWidth,
            expected: measure.endsWith('ch') ? Number.parseFloat(measure) * ch : Number.NaN,
            width: Number.parseFloat(style.width),
            sheet: sheet.clientWidth,
        };
    });
    expect(measured.maxWidth).not.toBe('none');
    expect(Math.abs(px(measured.maxWidth) - measured.expected)).toBeLessThan(2);
    expect(measured.width).toBeLessThan(measured.sheet);
});

test('the create page starts with the editor', async ({ page }) => {
    await page.setViewportSize(WIDE);
    const { share } = await create(page, {
        title: 'A title',
        items: [{ type: 'plain', text: 'title size' }],
    });
    const view = await openView(page, share);
    const titleSize = await view
        .locator('h1.cb-h1')
        .evaluate((element) => getComputedStyle(element).fontSize);
    await view.close();

    await fill(page, { items: [{ type: 'code', lang: 'rust', text: 'fn main() {}' }] });
    // Nothing visible before the form: the heading is for screen readers only, no lede.
    const headings = page.locator('main h1');
    await expect(headings).toHaveCount(1);
    await expect(headings).toHaveClass(/\bcb-sr-only\b/);
    await expect(headings).toHaveText(en['create.heading']!);
    await expect(page.locator('main .cb-lede')).toHaveCount(0);

    const title = page.getByLabel(en['create.titleLabel']!, { exact: true });
    await expect(title).toHaveAttribute('id', 'create-title');
    expect(await borderWidths(title)).toEqual([0, 0, 0, 0]);
    expect(await title.evaluate((element) => getComputedStyle(element).fontSize)).toBe(titleSize);

    const editor = page.locator('[data-editor]');
    for (const selector of [
        '[role="tablist"]',
        '#create-item-type',
        '#create-item-name',
        '#create-item-lang',
        '[data-item-text]',
        '[data-size-meter]',
    ]) {
        await expect(editor.locator(selector), selector).toHaveCount(1);
    }

    const text = page.locator('[data-item-text]');
    const ringOf = (locator: Locator) =>
        locator.evaluate((element) => {
            const style = getComputedStyle(element);
            return { shadow: style.boxShadow, border: style.borderTopColor };
        });
    // fill() leaves focus inside the frame; start from the title, outside it.
    await title.focus();
    const before = await ringOf(editor);
    await text.focus();
    await expect
        .poll(() => ringOf(editor), { message: 'the frame shows the focus ring' })
        .not.toEqual(before);
    expect(await borderWidths(text)).toEqual([0, 0, 0, 0]);
    expect(
        await text.evaluate((element) => {
            const style = getComputedStyle(element);
            return style.outlineStyle === 'none' || Number.parseFloat(style.outlineWidth) === 0;
        }),
    ).toBe(true);
});

test('the size bar fills with the share of the limit and turns danger-coloured over it', async ({
    page,
}) => {
    await page.setViewportSize(WIDE);
    await page.goto('/');
    await page.selectOption('#create-item-type', 'download');
    const bytes = Math.round(MAX_ITEM_BYTES * 0.3);
    await page.setInputFiles('[data-item-file]', {
        name: 'share.bin',
        mimeType: 'application/octet-stream',
        buffer: Buffer.alloc(bytes),
    });
    await expect(page.locator('[data-size-meter]')).toContainText('MiB of');
    const track = await box(page.locator('[data-size-bar]'));
    const filled = await box(page.locator('[data-size-fill]'));
    expect(Math.abs(filled.width / track.width - bytes / MAX_ITEM_BYTES)).toBeLessThanOrEqual(0.01);
    expect(await colour(page.locator('[data-size-fill]'), 'fill')).not.toBe(
        await token(page, '--cb-danger'),
    );

    await page.setInputFiles('[data-item-file]', {
        name: 'over.bin',
        mimeType: 'application/octet-stream',
        buffer: Buffer.alloc(MAX_ITEM_BYTES + 1),
    });
    await expect(page.locator('[data-size-meter]')).toContainText('over the limit');
    expect(await colour(page.locator('[data-size-fill]'), 'fill')).toBe(
        await token(page, '--cb-danger'),
    );
});

test('options share one row, the password hint follows the field, submit fits its label', async ({
    page,
}) => {
    await page.setViewportSize(WIDE);
    await page.goto('/');
    await expect(page.locator('[data-create-form]')).toBeVisible();

    const fields = {
        expiry: [page.locator('#create-expiry'), page.locator('label[for="create-expiry"]')],
        burn: [page.locator('label.cb-create-burn'), page.locator('#create-burn-label')],
        password: [page.locator('#create-password'), page.locator('label[for="create-password"]')],
    } as const;
    const tops = new Set<number>();
    for (const [name, [field, label]] of Object.entries(fields)) {
        const control = await box(field);
        const above = await box(label);
        tops.add(Math.round(control.y));
        expect(above.y + above.height, `${name}: label above`).toBeLessThanOrEqual(control.y + 0.5);
    }
    expect([...tops].length, 'one top offset').toBe(1);

    const note = page.locator('[data-password-note]');
    await expect(note).toBeHidden();
    await page.fill('#create-password', 'secret');
    await expect(note).toBeVisible();
    await expect(note).toHaveText(en['create.passwordNote']!);
    await page.fill('#create-password', '');
    await expect(note).toBeHidden();

    const form = page.locator('[data-create-form]');
    const submit = page.locator('[data-create-submit]');
    const trust = page.locator('[data-trust-line]');
    const button = await box(submit);
    expect(button.width).toBeLessThan((await box(form)).width / 2);
    // No wider than its content plus padding.
    expect(
        await submit.evaluate((element) => {
            const style = getComputedStyle(element);
            const range = document.createRange();
            range.selectNodeContents(element);
            const content = range.getBoundingClientRect().width;
            const padding =
                Number.parseFloat(style.paddingLeft) +
                Number.parseFloat(style.paddingRight) +
                Number.parseFloat(style.borderLeftWidth) +
                Number.parseFloat(style.borderRightWidth);
            return element.getBoundingClientRect().width - content - padding;
        }),
    ).toBeLessThanOrEqual(1);
    const line = await box(trust);
    expect(line.x).toBeGreaterThanOrEqual(button.x + button.width);
    expect(line.y).toBeLessThan(button.y + button.height);
    expect(line.y + line.height).toBeGreaterThan(button.y);

    await page.setViewportSize(NARROW);
    expect(Math.abs((await box(submit)).width - (await box(form)).width)).toBeLessThanOrEqual(1);
});
