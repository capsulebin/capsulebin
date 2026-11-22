import type { Page } from '@playwright/test';
import { type Capsule, create, fill } from './create';
import { catalogue } from './german';
import { expect, test } from './guard';

/**
 * Many items on the view and the create page: the "All N" list when the tabs do
 * not fit (every item in order, the same at every width, keyboard and Escape),
 * previous/next below the content, and on the create page the remove controls in
 * the list, "+ Item" beside it and "16 of 16" at the limit.
 */

const en = catalogue('en');

const NARROW = { width: 400, height: 800 };
const WIDE = { width: 1280, height: 800 };

const name = (index: number) => `item-${String(index + 1).padStart(2, '0')}.txt`;

/** Sixteen plain items, the most a capsule holds. */
const SIXTEEN: Capsule['items'] = Array.from({ length: 16 }, (_, index) => ({
    type: 'plain',
    name: name(index),
    text: `content of item ${index + 1}`,
}));

/** Text long enough that the page scrolls past an item's start. */
const long = (label: string) =>
    Array.from({ length: 120 }, (_, line) => `${label}, line ${line + 1}`).join('\n');

async function openView(page: Page, share: string): Promise<Page> {
    const view = await page.context().newPage();
    await view.setViewportSize(WIDE);
    await view.goto(share);
    await expect(view.locator('.cb-item-panel')).toBeVisible({ timeout: 30_000 });
    return view;
}

test('the view lists sixteen items under "All 16", the same at every width', async ({ page }) => {
    test.slow();
    await page.setViewportSize(WIDE);
    const { share } = await create(page, { items: SIXTEEN });
    const view = await openView(page, share);

    const all = view.locator('[data-items-all]');
    const menu = view.locator('#cb-all-menu');
    const rows = menu.locator('[role="menuitemradio"]');
    await expect(all).toBeVisible();
    await expect(all).toHaveText(en['items.all']!.replace('{count}', '16'));

    await all.click();
    await expect(menu).toBeVisible();
    await expect(rows).toHaveCount(16);
    await expect(rows.locator('.cb-itemtab-name')).toHaveText(SIXTEEN.map((item) => item.name!));
    await expect(rows.locator('.cb-itemtab-kind')).toHaveText(
        Array<string>(16).fill(en['item.badge.plain']!),
    );
    await expect(rows.locator('.cb-menu-size')).toHaveText(Array<RegExp>(16).fill(/^\d+ B$/));
    await expect(rows.nth(0)).toHaveAttribute('aria-checked', 'true');
    await expect(rows.nth(0)).toBeFocused();
    const wide = await rows.allTextContents();

    // Arrow keys move within the list, Escape closes it and returns to the button.
    await view.keyboard.press('ArrowDown');
    await expect(rows.nth(1)).toBeFocused();
    await view.keyboard.press('ArrowUp');
    await expect(rows.nth(0)).toBeFocused();
    await view.keyboard.press('Escape');
    await expect(menu).toBeHidden();
    await expect(all).toBeFocused();

    // A row selects its item, which becomes a visible tab.
    await all.click();
    await rows.nth(11).click();
    await expect(menu).toBeHidden();
    const tab = view.locator('#cb-item-tab-11');
    await expect(tab).toBeVisible();
    await expect(tab).toHaveAttribute('aria-selected', 'true');
    await expect(view.locator('.cb-item-panel')).toContainText('content of item 12');

    await view.setViewportSize(NARROW);
    await expect(all).toBeVisible();
    await all.click();
    await expect(rows).toHaveCount(16);
    expect(await rows.allTextContents()).toEqual(wide);
    await expect(rows.nth(11)).toHaveAttribute('aria-checked', 'true');
    await rows.nth(15).click();
    await expect(view.locator('#cb-item-tab-15')).toBeVisible();
    await expect(view.locator('#cb-item-tab-15')).toHaveAttribute('aria-selected', 'true');
});

test('previous and next move through the items below the content', async ({ page }) => {
    await page.setViewportSize(WIDE);
    const items: Capsule['items'] = [
        { type: 'plain', name: 'first.txt', text: long('first') },
        { type: 'plain', name: 'second.txt', text: long('second') },
        { type: 'plain', name: 'third.txt', text: long('third') },
    ];
    const { share } = await create(page, { items });
    const view = await openView(page, share);

    // Three tabs fit: no "All 3".
    await expect(view.locator('[data-items-all]')).toBeHidden();

    const sheet = view.locator('[data-sheet]');
    const nav = sheet.locator('.cb-item-nav');
    await expect(nav).toHaveCount(1);
    expect(
        await nav.evaluate(
            (row) => row.previousElementSibling?.classList.contains('cb-item-panel') ?? false,
        ),
        'the row follows the item panel',
    ).toBe(true);

    // A disabled end is also hidden (it keeps its place), so not found by its role.
    const previous = nav.locator('[data-item-previous]');
    const next = nav.locator('[data-item-next]');
    await expect(previous).toHaveAttribute('aria-label', en['items.previous']!);
    await expect(next).toHaveAttribute('aria-label', en['items.next']!);
    await expect(view.getByRole('button', { name: en['items.next']!, exact: true })).toBeVisible();
    const position = (index: number) =>
        en['items.position']!.replace('{index}', String(index)).replace('{count}', '3');
    await expect(previous).toBeDisabled();
    await expect(next).toBeEnabled();
    await expect(next).toHaveText('second.txt');
    await expect(nav).toContainText(position(1));

    // From the end of a long item, next shows the start of the following one
    // just below the sticky tab bar.
    await view.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
    await next.click();
    await expect(view.locator('#cb-item-tab-1')).toHaveAttribute('aria-selected', 'true');
    await expect(view.locator('.cb-item-panel')).toContainText('second, line 1');
    await expect(nav).toContainText(position(2));
    await expect(previous).toHaveText('first.txt');
    await expect(next).toHaveText('third.txt');
    const bar = await sheet.locator('.cb-sheet-bar').boundingBox();
    const panel = await view.locator('.cb-item-panel').boundingBox();
    expect(panel!.y).toBeGreaterThanOrEqual(bar!.y + bar!.height - 1);
    expect(panel!.y).toBeLessThanOrEqual(bar!.y + bar!.height + 24);

    await next.click();
    await expect(view.locator('#cb-item-tab-2')).toHaveAttribute('aria-selected', 'true');
    await expect(next).toBeDisabled();
    await previous.click();
    await expect(view.locator('#cb-item-tab-1')).toHaveAttribute('aria-selected', 'true');
});

test('one item has neither tabs nor previous/next', async ({ page }) => {
    const { share } = await create(page, { items: [{ type: 'plain', text: 'only one' }] });
    const view = await openView(page, share);
    await expect(view.locator('[data-sheet] .cb-item-nav')).toHaveCount(0);
    await expect(view.locator('[data-sheet] [role="tablist"]')).toHaveCount(0);
});

test('the create page lists sixteen items with remove controls, "+ Item" beside it', async ({
    page,
}) => {
    test.slow();
    await page.setViewportSize(WIDE);

    // One item: nothing to remove.
    await page.goto('/');
    await expect(page.locator('[data-create-form]')).toBeVisible();
    await expect(page.locator('[data-remove-id]')).toHaveCount(0);
    await expect(page.locator('[data-item-remove]')).toBeHidden();

    await fill(page, { items: SIXTEEN });
    const add = page.locator('[data-item-add]');
    const all = page.locator('[data-more-btn]');
    const menu = page.locator('[data-more-menu]');
    const rows = menu.locator('[role="menuitemradio"]');

    await expect(add).toBeDisabled();
    await expect(add).toHaveText(
        en['create.itemsFull']!.replace('{count}', '16').replace('{max}', '16'),
    );
    await expect(all).toBeVisible();
    await expect(all).toHaveText(en['items.all']!.replace('{count}', '16'));
    const expectAddRightOfAll = async () => {
        const left = (await all.boundingBox())!;
        const right = (await add.boundingBox())!;
        expect(right.x).toBeGreaterThanOrEqual(left.x + left.width);
    };
    await expectAddRightOfAll();

    await all.click();
    await expect(menu).toBeVisible();
    await expect(rows).toHaveCount(16);
    await expect(rows.locator('.cb-itemtab-name')).toHaveText(SIXTEEN.map((item) => item.name!));
    await expect(rows.locator('.cb-itemtab-kind')).toHaveText(
        Array<string>(16).fill(en['item.badge.plain']!),
    );
    for (let index = 0; index < 16; index++) {
        const remove = en['create.removeItem']!.replace('{name}', name(index));
        await expect(menu.getByRole('menuitem', { name: remove, exact: true })).toHaveCount(1);
    }
    await expect(rows.nth(15)).toHaveAttribute('aria-checked', 'true');
    await expect(rows.nth(15)).toBeFocused();
    const wide = await rows.allTextContents();

    // Arrow keys move through every control of the list, each row's remove control included.
    await page.keyboard.press('ArrowUp');
    await expect(
        menu.getByRole('menuitem', {
            name: en['create.removeItem']!.replace('{name}', name(14)),
            exact: true,
        }),
    ).toBeFocused();
    await page.keyboard.press('ArrowUp');
    await expect(rows.nth(14)).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
    await expect(all).toBeFocused();

    // A row selects its item for editing.
    await all.click();
    await rows.nth(2).click();
    await expect(menu).toBeHidden();
    const tab = page.locator('[data-item-tabs] .cb-itemtab[aria-selected="true"]');
    await expect(tab).toBeVisible();
    await expect(tab).toContainText(name(2));
    await expect(page.locator('[data-item-text]')).toHaveValue('content of item 3');

    await page.setViewportSize(NARROW);
    await expect(all).toBeVisible();
    await all.click();
    expect(await rows.allTextContents()).toEqual(wide);

    // A row's remove control removes that item; "+ Item" is back.
    const remove = en['create.removeItem']!.replace('{name}', name(4));
    await menu.getByRole('menuitem', { name: remove, exact: true }).click();
    await expect(add).toBeEnabled();
    await expect(add).toHaveText(en['create.addItem']!);
    await expect(all).toHaveText(en['items.all']!.replace('{count}', '15'));
    await expectAddRightOfAll();
    if (await menu.isHidden()) await all.click();
    await expect(rows).toHaveCount(15);
    expect(await rows.locator('.cb-itemtab-name').allTextContents()).not.toContain(name(4));
});
