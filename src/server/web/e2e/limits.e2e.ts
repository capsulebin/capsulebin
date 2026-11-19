import { createHash, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import type { Page } from '@playwright/test';
import { MAX_CAPSULE_BYTES, MAX_ITEM_BYTES, formatSize } from '@capsulebin/core';
import { expect, test } from './guard';

/**
 * The size limits in a real engine. A capsule at the item budget has to be packed,
 * padded, encrypted, sent as base64, accepted by the server's default
 * `pastes.max_plaintext_size`, fetched, decrypted and handed back as a download,
 * all in the browser's memory. One byte over the budget must be refused before
 * anything is sent, and a file over the absolute limit must not be read at all.
 */

const sha256 = (bytes: Uint8Array): string => createHash('sha256').update(bytes).digest('hex');

/** Opens the create page with a single, empty file item. */
async function openFileItem(page: Page): Promise<void> {
    await page.goto('/en/');
    await expect(page.locator('[data-create-form]')).toBeVisible();
    await page.selectOption('#create-item-type', 'download');
}

async function chooseFile(page: Page, name: string, bytes: Buffer): Promise<void> {
    await page.setInputFiles('[data-item-file]', {
        name,
        mimeType: 'application/octet-stream',
        buffer: bytes,
    });
}

test('a capsule at the size limit is created, opened and downloaded intact', async ({
    page,
    context,
}) => {
    // Packing, encrypting and base64 of 10 MiB is slow in some engines.
    test.slow();

    // Random bytes: incompressible, and any corruption changes the hash.
    const bytes = randomBytes(MAX_ITEM_BYTES);
    await openFileItem(page);
    await chooseFile(page, 'maximum.bin', bytes);

    const meter = page.locator('[data-size-meter]');
    const budget = formatSize(MAX_ITEM_BYTES);
    await expect(meter).toHaveText(`${budget} of ${budget}`);
    await expect(meter).not.toHaveClass(/cb-size-meter-over/);
    await page.click('[data-create-submit]');
    const share = page.locator('#share-link');
    await expect(share).not.toHaveValue('', { timeout: 60_000 });

    const view = await context.newPage();
    await view.goto(await share.inputValue());
    const panel = view.locator('.cb-item-panel');
    const button = panel.getByRole('button', { name: /download/i }).first();
    await expect(button).toBeVisible({ timeout: 60_000 });
    const [download] = await Promise.all([view.waitForEvent('download'), button.click()]);

    expect(download.suggestedFilename()).toBe('maximum.bin');
    const received = readFileSync(await download.path());
    expect(received.length).toBe(bytes.length);
    expect(sha256(received)).toBe(sha256(bytes));
});

test('one byte over the size limit cannot be submitted', async ({ page }) => {
    let requests = 0;
    page.on('request', (request) => {
        if (request.method() === 'POST' && request.url().includes('/api/v1/pastes')) requests++;
    });

    await openFileItem(page);
    await chooseFile(page, 'too-large.bin', Buffer.alloc(MAX_ITEM_BYTES + 1));

    const meter = page.locator('[data-size-meter]');
    await expect(meter).toHaveClass(/cb-size-meter-over/);
    await expect(meter).toContainText('over the limit by 1 B');
    await expect(page.locator('[data-create-submit]')).toBeDisabled();
    expect(requests).toBe(0);
});

test('a file over the absolute limit is refused before it is read', async ({ page }) => {
    await openFileItem(page);
    const dialog = page.waitForEvent('dialog');
    const choosing = chooseFile(page, 'huge.bin', Buffer.alloc(MAX_CAPSULE_BYTES + 1));
    const alert = await dialog;
    expect(alert.message()).toBe(`huge.bin exceeds the ${formatSize(MAX_CAPSULE_BYTES)} limit.`);
    await alert.dismiss();
    await choosing;

    // Nothing was added: the item is still the empty file item.
    await expect(page.locator('[data-size-meter]')).toHaveText(/^0 B of /);
    await expect(page.locator('.cb-itemtab')).toHaveCount(1);
});
