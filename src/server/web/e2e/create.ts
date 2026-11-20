import type { Page } from '@playwright/test';

import { DEFAULT_LANGUAGE } from '../src/default-language';
import { languagePrefix } from '../src/language-prefix';
import { expect } from './guard';

/** The create page as a user fills it in, shared by the flows in the `*.e2e.ts` files. */

/**
 * The path prefix of pages in `language` on the instance under test, whose default
 * language is the built-in one (playwright.config.ts sets none): none for English,
 * `/de` for German.
 */
export function prefixOf(language: string): string {
    return languagePrefix(DEFAULT_LANGUAGE, language);
}

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

/**
 * Creates a capsule through the create page in `language` and returns its two
 * links.
 */
export async function create(
    page: Page,
    capsule: Capsule,
    language = 'en',
): Promise<{ share: string; manage: string }> {
    await page.goto(`${prefixOf(language)}/`);
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
