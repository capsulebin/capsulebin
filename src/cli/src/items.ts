/**
 * Turns input files (or stdin) into capsule items: picks a presentation and,
 * for code, a language — the same four kinds the web editor offers.
 *
 * Inference is deliberately simple and overridable with `--as` / `--lang`:
 * Markdown by extension, code by an extension the web client can highlight,
 * any other valid UTF-8 text as plain, everything else as a download.
 */

import { generateUlid, type CapsuleItem } from '@capsulebin/core';
import { usage } from './exit';

export type Presentation = 'plain' | 'code' | 'markdown' | 'download';

/** `--as` values; `file` is the user-facing name for `download`. */
export const AS_VALUES: Record<string, Presentation> = {
    text: 'plain',
    plain: 'plain',
    code: 'code',
    markdown: 'markdown',
    md: 'markdown',
    file: 'download',
    download: 'download',
};

const MARKDOWN_EXTENSIONS = new Set(['md', 'markdown', 'mdown', 'mkd']);

/**
 * Extensions mapped to the highlight.js languages the web client registers
 * (`src/server/web/src/render-code.ts`). A language outside that allowlist
 * would still be stored, but render as plain text.
 */
const CODE_LANGUAGES: Record<string, string> = {
    sh: 'bash',
    bash: 'bash',
    zsh: 'bash',
    json: 'json',
    yml: 'yaml',
    yaml: 'yaml',
    ini: 'ini',
    toml: 'ini',
    cfg: 'ini',
    conf: 'ini',
    rs: 'rust',
    ts: 'typescript',
    tsx: 'typescript',
    mts: 'typescript',
    cts: 'typescript',
    js: 'javascript',
    jsx: 'javascript',
    mjs: 'javascript',
    cjs: 'javascript',
    py: 'python',
    sql: 'sql',
    html: 'html',
    htm: 'html',
    xml: 'html',
    css: 'css',
    diff: 'diff',
    patch: 'diff',
};

export const KNOWN_LANGUAGES = new Set([...Object.values(CODE_LANGUAGES), 'markdown', 'plaintext']);

export interface ItemInput {
    name: string;
    bytes: Uint8Array;
}

export interface ItemOptions {
    as?: Presentation | undefined;
    lang?: string | undefined;
}

function extension(name: string): string {
    const dot = name.lastIndexOf('.');
    return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

/** Valid UTF-8 without NUL bytes — the same test a user would apply by eye. */
export function isText(bytes: Uint8Array): boolean {
    if (bytes.includes(0)) return false;
    try {
        new TextDecoder('utf-8', { fatal: true }).decode(bytes);
        return true;
    } catch {
        return false;
    }
}

export function buildItem(input: ItemInput, options: ItemOptions): CapsuleItem {
    const ext = extension(input.name);
    const text = isText(input.bytes);

    let presentation: Presentation;
    if (options.as) {
        presentation = options.as;
    } else if (options.lang) {
        presentation = 'code';
    } else if (!text) {
        presentation = 'download';
    } else if (MARKDOWN_EXTENSIONS.has(ext)) {
        presentation = 'markdown';
    } else if (ext in CODE_LANGUAGES) {
        presentation = 'code';
    } else {
        presentation = 'plain';
    }

    if (presentation !== 'download' && !text) {
        throw usage(
            `${input.name}: not UTF-8 text, cannot be shared as ${presentation}; use --as file`,
        );
    }

    const language =
        presentation === 'code' ? (options.lang ?? CODE_LANGUAGES[ext] ?? 'plaintext') : undefined;

    const mediaType =
        presentation === 'download'
            ? 'application/octet-stream'
            : presentation === 'markdown'
              ? 'text/markdown'
              : 'text/plain';

    return {
        id: generateUlid(),
        name: input.name,
        size: input.bytes.length,
        mediaType,
        presentation,
        ...(language ? { language } : {}),
        bytes: input.bytes,
    };
}
