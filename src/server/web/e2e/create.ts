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

export interface Capsule {
    title?: string;
    items: Item[];
    password?: string;
    burn?: boolean;
    /** A value of the expiry select (seconds, `''` for none); its default if left out. */
    expiry?: string;
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
    await fill(page, capsule, language);
    await page.click('[data-create-submit]');
    return sealedLinks(page);
}

/** Opens the create page in `language` and fills it in with `capsule`, without submitting. */
export async function fill(page: Page, capsule: Capsule, language = 'en'): Promise<void> {
    await page.goto(`${prefixOf(language)}/`);
    await fillForm(page, capsule);
}

/** Fills in the create page `page` shows with `capsule`, without submitting. */
export async function fillForm(page: Page, capsule: Capsule): Promise<void> {
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
    if (capsule.expiry !== undefined) await page.selectOption('#create-expiry', capsule.expiry);
}

/**
 * The two links of the sealed page, once it shows them: the share link (with its
 * key) and the management link, as text of `[data-share-link]`/`[data-delete-link]`.
 */
export async function sealedLinks(
    page: Page,
    timeout = 30_000,
): Promise<{ share: string; manage: string }> {
    const share = page.locator('[data-share-link]');
    await expect(share).toHaveText(/#/, { timeout });
    return {
        share: (await share.textContent()) ?? '',
        manage: (await page.locator('[data-delete-link]').textContent()) ?? '',
    };
}
