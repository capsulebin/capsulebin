import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { expect } from './guard';

/** German pages without English left over, shared by the German flows of the `*.e2e.ts` files. */

/** The catalogue of `language` as the web build has it. */
export const catalogue = (language: string): Record<string, string> =>
    JSON.parse(
        readFileSync(new URL(`../src/i18n/${language}.json`, import.meta.url), 'utf8'),
    ) as Record<string, string>;

/**
 * English texts that must not appear on a German page: the literal parts of each
 * `en` value that differs from its `de` value, longer than 3 characters.
 *
 * Values with placeholders (`Expires in {count} minutes`) never appear verbatim,
 * so they are split at the placeholders and each literal part is checked on its
 * own; leaving them out would miss most error and status texts. A part that the
 * German value contains as well (`☰`, a unit) says nothing about the language.
 *
 * A part only counts as whole words (`Capsule` is in `CapsuleBin` on every page)
 * and case-insensitively, since `innerText` follows CSS `text-transform`.
 */
const ENGLISH_ONLY: RegExp[] = (() => {
    const en = catalogue('en');
    const de = catalogue('de');
    const parts = new Set<string>();
    for (const [key, value] of Object.entries(en)) {
        const german = de[key] ?? '';
        if (value === german) continue;
        for (const part of value.split(/\{\w+\}/)) {
            const literal = part.trim().replace(/\s+/g, ' ').toLowerCase();
            if (literal.length > 3 && !german.toLowerCase().includes(literal)) parts.add(literal);
        }
    }
    const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return [...parts].map(
        (part) => new RegExp(`(?<![\\p{L}\\p{N}])${escape(part)}(?![\\p{L}\\p{N}])`, 'iu'),
    );
})();

/**
 * The visible text of the page has German in it and nothing from the English catalogue.
 * `<code>` is left out: identifiers such as the storage key `capsulebin.language` are no language.
 */
export async function expectGerman(page: Page, expected: string): Promise<void> {
    const body = page.locator('body');
    await expect(body).toContainText(expected, { timeout: 30_000 });
    let text = await body.innerText();
    for (const code of await body.locator('code').allInnerTexts()) text = text.replace(code, ' ');
    text = text.replace(/\s+/g, ' ');
    expect(ENGLISH_ONLY.filter((english) => english.test(text)).map(String)).toEqual([]);
}
