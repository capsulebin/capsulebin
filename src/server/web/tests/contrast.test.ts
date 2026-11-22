// @vitest-environment node
//
// Reads tokens.css from disk and needs no DOM. Under jsdom `import.meta.url`
// would not be a file: URL.

/**
 * Contrast checks for the design tokens.
 *
 * The values are parsed out of tokens.css rather than restated here. A test that
 * repeats its own expectation gets edited together with the bug and then protects
 * nothing. The pairs and thresholds come from fixtures/contrast-pairs.json, which
 * the server's warnings for operator colours read as well.
 *
 * Checked against WCAG 2.2:
 *   - 1.4.3 Contrast (Minimum): text at least 4.5:1
 *   - 1.4.11 Non-text Contrast: control outlines at least 3:1
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const TOKENS_CSS = fileURLToPath(new URL('../src/styles/tokens.css', import.meta.url));
const CONTRAST_PAIRS = fileURLToPath(new URL('fixtures/contrast-pairs.json', import.meta.url));

/** Colour tokens of one block in tokens.css. */
type Palette = Record<string, string>;

/**
 * Reads the tokens of one selector block. Deliberately simple: the file is ours
 * and has no nested blocks inside a palette.
 */
function palette(css: string, selector: string): Palette {
    const start = css.indexOf(selector);
    if (start === -1) throw new Error(`selector ${selector} not found in tokens.css`);

    const open = css.indexOf('{', start);
    const close = css.indexOf('}', open);
    const block = css.slice(open + 1, close);

    const result: Palette = {};
    for (const line of block.split('\n')) {
        const match = /^\s*(--cb-[\w-]+):\s*(#[0-9a-fA-F]{6})\s*;/.exec(line);
        if (match?.[1] && match[2]) result[match[1]] = match[2].toLowerCase();
    }
    return result;
}

function channel(value: number): number {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function luminance(hex: string): number {
    const n = Number.parseInt(hex.slice(1), 16);
    const r = channel((n >> 16) & 0xff);
    const g = channel((n >> 8) & 0xff);
    const b = channel(n & 0xff);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string, b: string): number {
    const la = luminance(a);
    const lb = luminance(b);
    const hi = Math.max(la, lb);
    const lo = Math.min(la, lb);
    return (hi + 0.05) / (lo + 0.05);
}

const css = readFileSync(TOKENS_CSS, 'utf8');

/** The four blocks that each define a complete palette. */
const THEMES: ReadonlyArray<readonly [string, Palette]> = [
    ['light (:root)', palette(css, ':root {')],
    ['dark (media query)', palette(css, '@media (prefers-color-scheme: dark)')],
    ['light (data-theme)', palette(css, ':root[data-theme="light"]')],
    ['dark (data-theme)', palette(css, ':root[data-theme="dark"]')],
];

/**
 * The pairs and thresholds, shared with the server's contrast warnings for operator
 * colours (src/server/src/branding/colors.rs). Keys are config keys: `x_y` is the
 * token `--cb-x-y`.
 *
 * There are two border tokens because --cb-border is a divider and may be
 * restrained, while --cb-border-strong outlines controls and has to hold 3:1. As a
 * single token it would have to satisfy the stricter rule and the whole interface
 * would look needlessly heavy; so only --cb-border-strong appears in the fixture.
 */
interface ContrastPair {
    readonly foreground: readonly string[];
    readonly background: readonly string[];
    readonly min_ratio: number;
}

const { pairs: PAIRS } = JSON.parse(readFileSync(CONTRAST_PAIRS, 'utf8')) as {
    pairs: readonly ContrastPair[];
};

const token = (key: string) => `--cb-${key.replaceAll('_', '-')}`;

function need(p: Palette, name: string): string {
    const value = p[name];
    if (!value) throw new Error(`${name} missing from this palette`);
    return value;
}

describe.each(THEMES)('contrast in theme %s', (_name, p) => {
    describe.each(PAIRS)('pairs at $min_ratio:1', ({ foreground, background, min_ratio }) => {
        it.each(foreground)('%s reaches the threshold on every background', (key) => {
            for (const ground of background) {
                const ratio = contrast(need(p, token(key)), need(p, token(ground)));
                expect(
                    ratio,
                    `${token(key)} on ${token(ground)} = ${ratio.toFixed(2)}:1`,
                ).toBeGreaterThanOrEqual(min_ratio);
            }
        });
    });
});

/**
 * Depth: a surface (sheet, editor frame, link panel) has to stand out from the
 * page. In the light themes that is the ground's tint; #fbfcfc on #ffffff
 * (1.03:1) was not visible.
 */
describe('the light page background', () => {
    const light = THEMES.filter(([name]) => name.startsWith('light'));

    it.each(light)('is tinted against the surface in theme %s', (_name, p) => {
        const ratio = contrast(need(p, '--cb-bg'), need(p, '--cb-surface'));
        expect(ratio, `--cb-bg on --cb-surface = ${ratio.toFixed(3)}:1`).toBeGreaterThanOrEqual(
            1.06,
        );
    });

    it('is checked on both light blocks', () => {
        expect(light).toHaveLength(2);
    });
});

describe('structure of the token file', () => {
    /**
     * The manual toggle must be able to override the system preference in both
     * directions. Were one of the data-theme blocks missing, a light theme could not
     * be forced from a dark system.
     */
    it('defines both data-theme blocks with the same tokens as the default', () => {
        const base = Object.keys(palette(css, ':root {')).sort();

        for (const selector of [':root[data-theme="light"]', ':root[data-theme="dark"]']) {
            const keys = Object.keys(palette(css, selector)).sort();
            expect(keys, `${selector} has different tokens than :root`).toEqual(base);
        }
    });

    it('contains no web font reference', () => {
        expect(css).not.toMatch(/@font-face|fonts\.googleapis|\.woff/i);
    });
});
