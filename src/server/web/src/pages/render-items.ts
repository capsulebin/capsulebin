/**
 * Display conversion and rendering for decrypted capsule items: turning a
 * `CapsuleItem` (from `@capsulebin/core`) into what the page shows, and
 * rendering it as markdown, highlighted code, plain text or a download link.
 */

import { html, type TemplateResult } from 'lit';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';

import { type CapsuleItem, formatSize } from '@capsulebin/core';
import { highlightCode } from '../render-code';
import { renderMarkdown } from '../render-markdown';

export interface DisplayItem {
    text: string;
    presentation: string;
    language?: string;
    name?: string;
    bytes?: Uint8Array;
}

const KIND_BADGES: Record<string, string> = {
    plain: 'Text',
    code: 'Code',
    markdown: 'MD',
    download: 'File',
};

const KIND_LABELS: Record<string, string> = {
    plain: 'Text',
    code: 'Code',
    markdown: 'Markdown',
    download: 'File',
};

export function kindBadge(item: DisplayItem): string {
    return KIND_BADGES[item.presentation] || 'Item';
}

export function itemName(item: DisplayItem, index: number): string {
    if (item.name && item.name.length > 0) return item.name;
    return `${KIND_LABELS[item.presentation] || 'Item'} ${index + 1}`;
}

export function itemMeta(item: DisplayItem): string {
    const size = formatSize(item.bytes?.length ?? 0);
    const kind =
        item.presentation === 'code' && item.language
            ? item.language
            : KIND_LABELS[item.presentation] || 'Item';
    return `${kind} · ${size}`;
}

/**
 * Converts decoded capsule items into what the page displays: text decoded
 * eagerly for everything except downloads, which stay as raw bytes.
 */
export function toDisplayItems(items: readonly CapsuleItem[]): DisplayItem[] {
    return items.map((item) => {
        const isFile = item.presentation === 'download';
        const d: DisplayItem = {
            text: isFile ? '' : new TextDecoder().decode(item.bytes),
            presentation: item.presentation,
            name: item.name,
            bytes: item.bytes,
        };
        if (item.language) {
            d.language = item.language;
        }
        return d;
    });
}

/** File name for saving an item: its own name, or a generic one with a fitting
 *  extension. Never a path - the manifest name is display metadata. */
export function downloadName(item: DisplayItem, index: number): string {
    const base = (item.name ?? '').split(/[\\/]/).at(-1)?.trim();
    if (base) return base;
    const extension =
        item.presentation === 'markdown' ? 'md' : item.presentation === 'download' ? 'bin' : 'txt';
    return `item-${index + 1}.${extension}`;
}

/**
 * Saves an item's bytes as a file. A fresh object URL per click, revoked once
 * the browser has picked it up - a URL rendered into the page and revoked after
 * the first click would make every later click fail.
 */
export function downloadItem(item: DisplayItem, index: number): void {
    const blob = new Blob([(item.bytes ?? new TextEncoder().encode(item.text)) as BlobPart], {
        type: 'application/octet-stream',
    });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = downloadName(item, index);
    anchor.rel = 'noopener';
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export interface RenderOptions {
    /** Show Markdown as its source instead of rendered. */
    source: boolean;
    index: number;
}

export function renderItemContent(item: DisplayItem, options: RenderOptions): TemplateResult {
    if (item.presentation === 'markdown' && !options.source) {
        // unsafeHTML wants a plain string, not the app's own TrustedHTML-branded
        // value: lit-html re-brands it through its own 'lit-html' Trusted Types
        // policy right before the real innerHTML assignment, so trustedHtml()
        // (for this project's own innerHTML sinks) does not apply here.
        return html`<div class="cb-card">
            <div class="cb-prose">${unsafeHTML(renderMarkdown(item.text))}</div>
        </div>`;
    }
    if (item.presentation === 'code') {
        return html`<div class="cb-card">
            <pre
                class="cb-code-block"
            ><code>${unsafeHTML(highlightCode(item.text, item.language || 'plaintext'))}</code></pre>
        </div>`;
    }
    if (item.presentation === 'download') {
        return html`<div class="cb-card cb-file-card">
            <p class="cb-lede">
                Files are never previewed. It was decrypted in your browser and is saved exactly as
                the sender added it.
            </p>
            <button type="button" class="cb-btn" @click=${() => downloadItem(item, options.index)}>
                Download ${downloadName(item, options.index)} (${formatSize(item.bytes?.length ?? 0)})
            </button>
        </div>`;
    }
    // plain, Markdown source, or unknown — shown as text, never interpreted.
    return html`<div class="cb-card"><pre class="cb-pre">${item.text}</pre></div>`;
}
