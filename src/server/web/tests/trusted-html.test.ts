/**
 * Behavioural tests for the Trusted Types policy wrapper. The companion
 * static-scan test that every `innerHTML` sink actually calls it lives in
 * `tests/trusted-html-usage.test.ts` (needs the `node` environment, this file
 * needs `window`).
 *
 * @see docs/threat-model.md (XSS through Markdown or code)
 */

import { afterEach, describe, expect, it, vi } from 'vitest';

describe('trustedHtml', () => {
    afterEach(() => {
        vi.unstubAllGlobals();
        vi.resetModules();
    });

    it('returns the string unchanged when the browser has no Trusted Types support', async () => {
        // jsdom does not implement `window.trustedTypes` - this is the Firefox/Safari case.
        expect(window.trustedTypes).toBeUndefined();

        const { trustedHtml } = await import('../src/trusted-html');
        expect(trustedHtml('<p>hi</p>')).toBe('<p>hi</p>');
    });

    it('routes through a Trusted Types policy when the browser enforces one', async () => {
        const createHTML = vi.fn((html: string) => `branded:${html}`);
        const createPolicy = vi.fn().mockReturnValue({ createHTML });
        vi.stubGlobal('trustedTypes', { createPolicy });

        // The policy is created at module load, so it must be imported after the stub is in place.
        const { trustedHtml } = await import('../src/trusted-html');

        expect(createPolicy).toHaveBeenCalledWith('capsulebin-html', expect.any(Object));
        expect(trustedHtml('<p>hi</p>')).toBe('branded:<p>hi</p>');
    });
});
