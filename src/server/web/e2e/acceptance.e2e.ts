import type { Locator, Page } from '@playwright/test';
import { create, fill } from './create';
import { catalogue } from './german';
import { expect, test } from './guard';
import { colour, settled, token } from './style';

/**
 * Acceptance checks of the refreshed view and create pages in a real engine,
 * derived from the criteria rather than the implementation: content without
 * borders or shadows on the sheet, the warning-coloured burn chip first in the
 * chip row, the code gutter kept out of selection, and the item actions with
 * their labels at a wide viewport.
 */

const en = catalogue('en');

const WIDE = { width: 1280, height: 800 };

async function openView(page: Page, share: string): Promise<Page> {
    const view = await page.context().newPage();
    await view.setViewportSize(WIDE);
    await view.goto(share);
    return view;
}

/**
 * Every rendered content element inside `root` (the root included) with a border
 * or a box shadow. Controls (a file card's download button) keep their control
 * style and are not content surfaces.
 */
async function framed(root: Locator): Promise<string[]> {
    return root.evaluate((root) =>
        [root, ...root.querySelectorAll('*')]
            .filter((element) => !element.closest('button, input, select, textarea'))
            .filter((element) => {
                const style = getComputedStyle(element);
                const border = [
                    style.borderTopWidth,
                    style.borderRightWidth,
                    style.borderBottomWidth,
                    style.borderLeftWidth,
                ].some((width) => Number.parseFloat(width) !== 0);
                return border || style.boxShadow !== 'none';
            })
            .map((element) => `${element.localName}.${[...element.classList].join('.')}`),
    );
}

test('no element inside the item panel has a border or a shadow, for every kind', async ({
    page,
}) => {
    const { share } = await create(page, {
        items: [
            { type: 'plain', name: 'plain.txt', text: 'just text\nover two lines' },
            {
                type: 'markdown',
                name: 'prose.md',
                text: '# Heading\n\nA *paragraph* with `inline code`.\n\n- one\n- two\n\n```\nfenced block\n```\n',
            },
            { type: 'code', name: 'main.rs', lang: 'rust', text: 'fn main() {\n    // hi\n}\n' },
            { type: 'download', name: 'blob.bin', bytes: [0, 1, 2, 3] },
        ],
    });
    const view = await openView(page, share);
    const panel = view.locator('[data-sheet] .cb-item-panel');
    await expect(panel).toBeVisible({ timeout: 30_000 });

    const tabs = view.locator('[data-sheet] [role="tablist"] [role="tab"]');
    await expect(tabs).toHaveCount(4);
    for (let index = 0; index < 4; index++) {
        await view.locator(`#cb-item-tab-${index}`).click();
        await expect(view.locator(`#cb-item-tab-${index}`)).toHaveAttribute(
            'aria-selected',
            'true',
        );
        await settled(view);
        expect(await framed(panel), `item ${index}`).toEqual([]);
        expect(await view.locator('.cb-card').count(), `item ${index}`).toBe(0);
    }
    // The Markdown source view too.
    await view.locator('#cb-item-tab-1').click();
    await view.locator('.cb-view-switch button').nth(1).click();
    await expect(panel.locator('pre')).toBeVisible();
    expect(await framed(panel), 'Markdown source').toEqual([]);
});

for (const colorScheme of ['light', 'dark'] as const) {
    test(`the burn chip comes first, in the warning colour (${colorScheme})`, async ({ page }) => {
        const { share } = await create(page, {
            burn: true,
            items: [{ type: 'plain', text: 'burn chip' }],
        });
        const view = await page.context().newPage();
        await view.emulateMedia({ colorScheme });
        await view.goto(share);
        await view
            .getByRole('button', { name: en['view.burnConfirm']!, exact: true })
            .click({ timeout: 30_000 });
        const chips = view.locator('.cb-chips > li');
        await expect(chips.first()).toBeVisible({ timeout: 30_000 });
        await expect(view.locator('.cb-chips')).toHaveCount(1);
        await expect(chips.first()).toHaveText(en['view.deletedAfterRetrieval']!);
        expect(await colour(chips.first(), 'color')).toBe(await token(view, '--cb-warning'));
        await expect(chips.last()).toHaveText(en['view.decryptedLocally']!);
        // Between the title and the sheet.
        const order = await view.evaluate(() => {
            const h1 = document.querySelector('main h1')!;
            const row = document.querySelector('.cb-chips')!;
            const sheet = document.querySelector('[data-sheet]')!;
            return (
                Boolean(h1.compareDocumentPosition(row) & Node.DOCUMENT_POSITION_FOLLOWING) &&
                Boolean(row.compareDocumentPosition(sheet) & Node.DOCUMENT_POSITION_FOLLOWING)
            );
        });
        expect(order).toBe(true);
        await expect(view.locator('.cb-capsule-status')).toHaveCount(0);
    });
}

test('the code gutter is hidden from assistive technology and from selection', async ({ page }) => {
    const text = 'fn main() {\n    /* a\n       b */\n}\n';
    const { share } = await create(page, {
        items: [
            { type: 'code', name: 'main.rs', lang: 'rust', text },
            { type: 'plain', name: 'plain.txt', text: 'a\nb\nc' },
        ],
    });
    const view = await openView(page, share);
    const gutter = view.locator('.cb-item-panel [aria-hidden="true"]', { hasText: /^1\s+2/ });
    await expect(gutter).toBeVisible({ timeout: 30_000 });
    expect(await gutter.evaluate((element) => getComputedStyle(element).userSelect)).toBe('none');
    expect((await gutter.innerText()).split(/\s+/).filter(Boolean).map(Number)).toEqual([
        1, 2, 3, 4,
    ]);
    const code = view.locator('.cb-item-panel code');
    expect(await code.evaluate((element) => element.textContent)).toBe(text);
    expect(
        await gutter.evaluate(
            (element, code) => element.contains(code),
            await code.elementHandle(),
        ),
    ).toBe(false);
    await expect(view.locator('.cb-item-panel .hljs-comment')).toHaveCount(1);

    await view.locator('#cb-item-tab-1').click();
    await expect(view.locator('.cb-item-panel')).toContainText('a');
    await expect(view.locator('.cb-item-panel .cb-code-gutter')).toHaveCount(0);
    await expect(
        view.locator('.cb-item-panel [aria-hidden="true"]', { hasText: /^1/ }),
    ).toHaveCount(0);
});

test('at 1280 px the create page item actions show icon and label', async ({ page }) => {
    await page.setViewportSize(WIDE);
    await fill(page, {
        items: [
            { type: 'plain', name: 'a.txt', text: 'a' },
            { type: 'plain', name: 'b.txt', text: 'b' },
            { type: 'plain', name: 'c.txt', text: 'c' },
        ],
    });
    await settled(page);
    const buttons = page.locator('[data-item-actions] button:visible');
    expect(await buttons.count()).toBeGreaterThan(0);
    for (const button of await buttons.all()) {
        await expect(button.locator('svg')).toBeVisible();
        const label = button.locator('.cb-bar-label');
        if ((await label.count()) === 0) continue;
        expect((await label.boundingBox())!.width).toBeGreaterThan(1);
    }
    await expect(page.locator('[data-item-actions]')).toContainText(en['create.remove']!);
});
