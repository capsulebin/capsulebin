import type { Page } from '@playwright/test';
import { fillForm, openCreate } from './create';
import { expect, test } from './guard';
import { box, expectNoHorizontalScroll, px, settled } from './style';

/**
 * The create page's editor in a real engine: it fills the window down to the
 * submit row and keeps a long text reachable, shows no tab bar while there is
 * one item (also on a phone), and keeps its head and size footer off the
 * frame's edge.
 */

const WIDE = { width: 1280, height: 800 };
const TALL = { width: 1280, height: 1000 };
const NARROW = { width: 400, height: 700 };

/** With one item: no tab bar, the name only in its field, the add button in the item row. */
async function expectNoTabBar(page: Page, name: string): Promise<void> {
    const editor = page.locator('[data-editor]');
    const tabs = page.locator('[data-item-tabs] [role="tab"]');
    const add = page.locator('[data-item-add]');
    await expect(page.locator('[data-tabs-row]')).toBeHidden();
    await expect(tabs).toHaveCount(1);
    await expect(editor.locator('[role="tab"]').filter({ visible: true })).toHaveCount(0);
    await expect(page.locator('[data-more-btn]')).toBeHidden();
    await expect(page.locator('#create-item-name')).toHaveValue(name);
    // The tab stays in the DOM (it labels the panel), but nothing shows the name as text.
    await expect(editor.getByText(name).filter({ visible: true })).toHaveCount(0);
    await expect(add).toBeVisible();
    await expect(add).toBeEnabled();
    await expect(page.locator('[data-item-actions] [data-item-add]')).toHaveCount(1);
    await expect(editor.locator('[data-item-add]')).toHaveCount(1);
}

test('the editor fills the window down to the submit button', async ({ page }) => {
    const submit = page.locator('[data-create-submit]');
    const heights: number[] = [];
    for (const viewport of [WIDE, TALL]) {
        await page.setViewportSize(viewport);
        await openCreate(page);
        const button = await box(submit);
        const bottom = button.y + button.height;
        const where = `${viewport.width}×${viewport.height}`;
        expect(await page.evaluate(() => window.scrollY), where).toBe(0);
        expect(bottom, `${where}: inside the viewport`).toBeLessThanOrEqual(viewport.height);
        expect(viewport.height - bottom, `${where}: near its bottom`).toBeLessThanOrEqual(64);
        heights.push((await box(page.locator('[data-editor]'))).height);
    }
    // The frame takes the extra height, nothing else does.
    expect(Math.abs(heights[1]! - heights[0]! - (TALL.height - WIDE.height))).toBeLessThanOrEqual(
        1,
    );

    await page.setViewportSize(NARROW);
    await openCreate(page);
    const minimum = await page.evaluate(
        () => 15 * Number.parseFloat(getComputedStyle(document.documentElement).fontSize),
    );
    expect((await box(page.locator('[data-item-text]'))).height).toBeGreaterThanOrEqual(
        minimum - 0.5,
    );
    const scroll = await page.evaluate(() => ({
        content: document.documentElement.scrollHeight,
        window: document.documentElement.clientHeight,
    }));
    expect(scroll.content, 'the page scrolls').toBeGreaterThan(scroll.window);
});

test('more lines than fit scroll inside the text area or grow it, and nothing is lost', async ({
    page,
}) => {
    await page.setViewportSize(WIDE);
    await openCreate(page);
    const textarea = page.locator('[data-item-text]');
    const before = await box(textarea);

    const text = Array.from({ length: 300 }, (_, index) => `line ${index + 1}`).join('\n');
    await textarea.fill(text);
    await settled(page);

    // Scrolling inside is measured by behaviour: the engines do not agree on the
    // computed `overflow-y` of a text area.
    const after = await textarea.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
        const scrolled = element.scrollTop;
        element.scrollTop = 0;
        return {
            height: element.getBoundingClientRect().height,
            client: element.clientHeight,
            scroll: element.scrollHeight,
            scrolled,
        };
    });
    const scrollsInside = after.scroll > after.client + 1 && after.scrolled > 0;
    const grew = after.height > before.height + 1 && after.scroll <= after.client + 1;
    expect(scrollsInside || grew, JSON.stringify(after)).toBe(true);
    // Never smaller than the empty editor, which filled the window.
    expect(after.height).toBeGreaterThanOrEqual(before.height - 1);
    await expect(textarea).toHaveValue(text);

    // The last line can be reached: the caret at the end is scrolled into view.
    await textarea.press('Control+End');
    await textarea.pressSequentially('!');
    await expect(textarea).toHaveValue(`${text}!`);
    if (scrollsInside) {
        expect(await textarea.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    }

    // The submit button stays reachable and the page does not scroll sideways.
    const submit = page.locator('[data-create-submit]');
    await submit.scrollIntoViewIfNeeded();
    await expect(submit).toBeInViewport();
    await expectNoHorizontalScroll(page);
});

test('one item has no tab bar, two or more bring it back', async ({ page }) => {
    await page.setViewportSize(WIDE);
    await openCreate(page);
    const bar = page.locator('[data-tabs-row]');
    const tabs = page.locator('[data-item-tabs] [role="tab"]');
    const add = page.locator('[data-item-add]');

    await page.fill('#create-item-name', 'only.txt');
    await page.fill('[data-item-text]', 'the only item');
    await expectNoTabBar(page, 'only.txt');

    // The add button works from the item row; the tab bar returns with both items.
    await add.click();
    await page.click('[data-add-type="markdown"]');
    await page.fill('#create-item-name', 'second.md');
    await expect(bar).toBeVisible();
    await expect(tabs).toHaveCount(2);
    for (const [index, name] of ['only.txt', 'second.md'].entries()) {
        await expect(tabs.nth(index)).toBeVisible();
        await expect(tabs.nth(index)).toContainText(name);
    }
    await expect(page.locator('[data-tabs-row] [data-item-add]')).toHaveCount(1);
    await expect(page.locator('[data-item-actions] [data-item-add]')).toHaveCount(0);
    await expect(add).toBeVisible();

    // Back to one item: hidden again, the first item's content kept.
    await page.click('[data-item-remove]');
    await expectNoTabBar(page, 'only.txt');
    await expect(page.locator('[data-item-text]')).toHaveValue('the only item');
    await add.click();
    await expect(page.locator('[data-add-menu]')).toBeVisible();
});

test('one item on a phone: no tab bar, the name once, the add button in reach', async ({
    page,
}) => {
    await page.setViewportSize(NARROW);
    await openCreate(page);
    const editor = page.locator('[data-editor]');
    const add = page.locator('[data-item-add]');

    await page.fill('#create-item-name', 'only.txt');
    await page.fill('[data-item-text]', 'the only item');
    await expectNoTabBar(page, 'only.txt');

    // The name is shown once in the editor: as the value of its field, nowhere as text.
    const fields = await editor.locator('input, select, textarea').evaluateAll(
        (controls) =>
            controls.filter((control) => {
                const rect = control.getBoundingClientRect();
                return (
                    rect.width > 0 &&
                    rect.height > 0 &&
                    (control as HTMLInputElement).value === 'only.txt'
                );
            }).length,
    );
    expect(fields).toBe(1);

    // The add button lies inside the frame and the window, and opens its menu.
    const frame = await box(editor);
    const button = await box(add);
    expect(button.x).toBeGreaterThanOrEqual(frame.x);
    expect(button.x + button.width).toBeLessThanOrEqual(frame.x + frame.width);
    expect(button.x + button.width).toBeLessThanOrEqual(NARROW.width);
    await expectNoHorizontalScroll(page);

    await add.click();
    await expect(page.locator('[data-add-menu]')).toBeVisible();
    await page.click('[data-add-type="markdown"]');
    await expect(page.locator('[data-tabs-row]')).toBeVisible();
    await expect(page.locator('[data-item-tabs] [role="tab"]')).toHaveCount(2);
    // The first item kept its content when the tab bar came back. At this width its
    // tab may be in the "All 2" list, so it is chosen in a wide window.
    await page.setViewportSize(WIDE);
    await page.locator('[data-item-tabs] [role="tab"]').first().click();
    await expect(page.locator('[data-item-text]')).toHaveValue('the only item');
});

test('"All N" returns with many items and leaves with the tab bar', async ({ page }) => {
    await page.setViewportSize({ width: 400, height: 800 });
    await page.goto('/');
    await fillForm(page, {
        items: Array.from({ length: 6 }, (_, index) => ({
            type: 'plain' as const,
            name: `a-rather-long-item-name-${index + 1}.txt`,
            text: `item ${index + 1}`,
        })),
    });
    const all = page.locator('[data-more-btn]');
    await expect(page.locator('[data-tabs-row]')).toBeVisible();
    await expect(all).toBeVisible();

    for (let left = 5; left >= 1; left--) {
        await page.click('[data-item-remove]');
        await expect(page.locator('[data-item-tabs] [role="tab"]')).toHaveCount(left);
    }
    await expect(page.locator('[data-tabs-row]')).toBeHidden();
    await expect(all).toBeHidden();
});

for (const viewport of [WIDE, NARROW]) {
    test(`the editor head and size footer keep off the frame at ${viewport.width} px`, async ({
        page,
    }) => {
        await page.setViewportSize(viewport);
        await openCreate(page);

        const expectRoom = async (where: string) => {
            await settled(page);
            for (const selector of ['[data-editor-head]', '.cb-create-foot']) {
                const padding = await page.locator(selector).evaluate((element) => {
                    const style = getComputedStyle(element);
                    return [style.paddingTop, style.paddingBottom];
                });
                for (const value of padding) {
                    expect(px(value), `${where}: ${selector}`).toBeGreaterThanOrEqual(12);
                }
            }
            // Every control of the head, measured against the frame's outer edge.
            const distances = await page.locator('[data-editor]').evaluate((frame) => {
                const outer = frame.getBoundingClientRect();
                const head = frame.querySelector('[data-editor-head]')!;
                return [...head.querySelectorAll('button, input, select')]
                    .map((control) => control.getBoundingClientRect())
                    .filter((inner) => inner.width > 0 && inner.height > 0)
                    .map((inner) =>
                        Math.min(
                            inner.left - outer.left,
                            outer.right - inner.right,
                            inner.top - outer.top,
                        ),
                    );
            });
            expect(distances.length, where).toBeGreaterThan(0);
            for (const distance of distances) {
                expect(distance, `${where}: control to frame edge`).toBeGreaterThanOrEqual(10);
            }
        };

        await expectRoom('one item');
        await fillForm(page, {
            items: [
                { type: 'plain', name: 'first.txt', text: 'first' },
                { type: 'markdown', name: 'second.md', text: '# Second' },
                { type: 'code', name: 'third.rs', lang: 'rust', text: 'fn main() {}' },
            ],
        });
        await expectRoom('three items');
    });
}
