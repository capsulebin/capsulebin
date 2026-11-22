// @vitest-environment node
//
// Reads the stylesheets, the public files and (if built) dist from disk; no DOM.

/**
 * Fonts: system stacks only. The Content Security Policy allows no external
 * resource and the build ships no font, so the monospace stack names the coding
 * fonts a reader may have installed before each platform's own face.
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const dir = (path: string) => fileURLToPath(new URL(path, import.meta.url));

const STYLES = dir('../src/styles');
const PUBLIC = dir('../public');
const DIST = dir('../dist');

/** In this order, between `ui-monospace` and the generic `monospace`. */
const MONO_FAMILIES = [
    'JetBrains Mono',
    'Cascadia Code',
    'Cascadia Mono',
    'Fira Code',
    'Fira Mono',
    'Source Code Pro',
    'SF Mono',
    'SFMono-Regular',
    'Menlo',
    'Consolas',
    'DejaVu Sans Mono',
    'Liberation Mono',
] as const;

const FONT_FILE = new Set(['.woff', '.woff2', '.ttf', '.otf', '.eot']);

/** Every file below `root`, as a path. */
function files(root: string): string[] {
    return readdirSync(root, { recursive: true, withFileTypes: true })
        .filter((entry) => entry.isFile())
        .map((entry) => join(entry.parentPath, entry.name));
}

/** The families of `--cb-font-mono`, unquoted, in their order. */
function monoStack(): string[] {
    const tokens = readFileSync(join(STYLES, 'tokens.css'), 'utf8');
    const match = /--cb-font-mono:([^;]+);/.exec(tokens);
    if (!match?.[1]) throw new Error('--cb-font-mono not found in tokens.css');
    return match[1].split(',').map((family) => family.trim().replace(/^"(.*)"$/, '$1'));
}

describe('the monospace stack', () => {
    it('starts with ui-monospace and ends with the generic family', () => {
        const stack = monoStack();
        expect(stack[0]).toBe('ui-monospace');
        expect(stack.at(-1)).toBe('monospace');
    });

    it('names the coding fonts and every platform face before the generic family', () => {
        expect(monoStack().slice(1, -1)).toEqual(MONO_FAMILIES);
    });
});

describe.each([
    ['the sources', [STYLES, PUBLIC]],
    ['the built pages', [DIST]],
])('no embedded font in %s', (_name, roots) => {
    // dist exists only after a build; the sources are always checked.
    const present = roots.filter((root) => existsSync(root)).flatMap(files);

    it('has no font file', () => {
        expect(present.filter((path) => FONT_FILE.has(extname(path).toLowerCase()))).toEqual([]);
    });

    it('has no @font-face rule', () => {
        const stylesheets = present.filter((path) => extname(path) === '.css');
        for (const path of stylesheets) {
            expect(readFileSync(path, 'utf8'), path).not.toMatch(/@font-face/i);
        }
    });
});
