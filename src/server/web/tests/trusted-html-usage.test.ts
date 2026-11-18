// @vitest-environment node
//
// Scans source files on disk; needs no DOM.

/**
 * Guards the CSP's `require-trusted-types-for 'script'` directive
 * (`src/server/src/api/security_headers.rs`) against a
 * regression: a bare `el.innerHTML = someString` passes every other test in
 * this suite, because jsdom does not enforce Trusted Types at all, and then
 * throws in every Chromium-based browser — exactly what shipped before
 * `src/trusted-html.ts` was introduced.
 *
 * Lit's `unsafeHTML(...)` directive is a different kind of sink: it wants a
 * plain JS string, not the app's own `TrustedHTML`-branded value — lit-html
 * re-brands it through its own built-in `'lit-html'` Trusted Types policy
 * right before the real `innerHTML` assignment. What still must hold is that
 * the string is pre-sanitized, i.e. it came out of `renderMarkdown(...)` or
 * `highlightCode(...)`, never a raw value handed to `unsafeHTML` directly.
 *
 * @see docs/threat-model.md (XSS through Markdown or code)
 */

import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

function sourceFiles(): { path: string; content: string }[] {
    const dir = fileURLToPath(new URL('../src', import.meta.url));
    const out: { path: string; content: string }[] = [];

    for (const entry of readdirSync(dir, { withFileTypes: true, recursive: true })) {
        if (!entry.isFile() || !entry.name.endsWith('.ts')) continue;
        const path = `${entry.parentPath}/${entry.name}`;
        if (path.endsWith('/trusted-html.ts')) continue; // the policy itself
        out.push({ path, content: readFileSync(path, 'utf8') });
    }

    return out;
}

describe('innerHTML sinks', () => {
    it('every innerHTML assignment in src/ is wrapped in trustedHtml(...)', () => {
        const offenders: string[] = [];

        for (const { path, content } of sourceFiles()) {
            for (const line of content.split('\n')) {
                if (!/\.innerHTML\s*=/.test(line)) continue;
                if (/\.innerHTML\s*=\s*trustedHtml\(/.test(line)) continue;
                offenders.push(`${path}: ${line.trim()}`);
            }
        }

        expect(offenders, 'found an innerHTML assignment that bypasses trusted-html.ts').toEqual(
            [],
        );
    });

    it('sanity check: the scan actually finds an unwrapped assignment', () => {
        // Proves the regex above is not vacuously true.
        const files = sourceFiles();
        expect(files.length).toBeGreaterThan(0);

        const line = 'el.innerHTML = "<p>not wrapped</p>";';
        const isOffender =
            /\.innerHTML\s*=/.test(line) && !/\.innerHTML\s*=\s*trustedHtml\(/.test(line);
        expect(isOffender).toBe(true);
    });

    it('every unsafeHTML(...) call in src/ wraps renderMarkdown(...) or highlightCode(...)', () => {
        const offenders: string[] = [];

        for (const { path, content } of sourceFiles()) {
            for (const line of content.split('\n')) {
                if (!/\bunsafeHTML\(/.test(line)) continue;
                if (/\bunsafeHTML\(\s*(renderMarkdown|highlightCode)\(/.test(line)) continue;
                offenders.push(`${path}: ${line.trim()}`);
            }
        }

        expect(offenders, 'found an unsafeHTML(...) call not fed by a sanitizing function').toEqual(
            [],
        );
    });

    it('sanity check: the scan actually finds an unwrapped unsafeHTML(...) call', () => {
        const line = 'html`${unsafeHTML(someRawString)}`;';
        const isOffender =
            /\bunsafeHTML\(/.test(line) &&
            !/\bunsafeHTML\(\s*(renderMarkdown|highlightCode)\(/.test(line);
        expect(isOffender).toBe(true);
    });
});
