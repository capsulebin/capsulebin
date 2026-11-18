/**
 * Tests for the theme toggle.
 *
 * Most important point: `localStorage` may only ever see the theme choice.
 * Anything else - fragment, password, key material, plaintext - would defeat the
 * zero-knowledge property through a persistence side channel.
 */

import { beforeEach, describe, expect, it } from 'vitest';

import { applyChoice, effectiveTheme, mountThemeToggle, readChoice } from '../src/theme';

const STORAGE_KEY = 'capsulebin.theme';

function fakeMatchMedia(prefersDark: boolean): void {
    Object.defineProperty(window, 'matchMedia', {
        writable: true,
        value: (query: string) => ({
            matches: query.includes('dark') ? prefersDark : false,
            media: query,
            addEventListener: () => {},
            removeEventListener: () => {},
        }),
    });
}

describe('theme choice', () => {
    beforeEach(() => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
        fakeMatchMedia(false);
    });

    it('is auto when nothing is stored', () => {
        expect(readChoice()).toBe('auto');
    });

    it('ignores an unknown stored value', () => {
        localStorage.setItem(STORAGE_KEY, 'neon');
        expect(readChoice()).toBe('auto');
    });

    it('removes data-theme for auto so the media query applies', () => {
        applyChoice('dark');
        expect(document.documentElement.dataset.theme).toBe('dark');

        applyChoice('auto');
        expect(document.documentElement.dataset.theme).toBeUndefined();
    });

    it('follows the system preference when set to auto', () => {
        fakeMatchMedia(true);
        expect(effectiveTheme('auto')).toBe('dark');

        fakeMatchMedia(false);
        expect(effectiveTheme('auto')).toBe('light');
    });

    it('overrides the system preference in both directions', () => {
        fakeMatchMedia(true);
        expect(effectiveTheme('light')).toBe('light');

        fakeMatchMedia(false);
        expect(effectiveTheme('dark')).toBe('dark');
    });
});

describe('toggle', () => {
    beforeEach(() => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
        fakeMatchMedia(false);
    });

    /**
     * The order auto -> light -> dark -> auto is deliberately three-stage: were it
     * only light vs. dark, the system preference would be unreachable after the
     * first click.
     */
    it('cycles through auto, light, dark and back to auto', () => {
        const button = document.createElement('button');
        mountThemeToggle(button);

        expect(button.dataset.theme).toBe('auto');

        button.click();
        expect(button.dataset.theme).toBe('light');
        expect(document.documentElement.dataset.theme).toBe('light');

        button.click();
        expect(button.dataset.theme).toBe('dark');

        button.click();
        expect(button.dataset.theme).toBe('auto');
        expect(document.documentElement.dataset.theme).toBeUndefined();
        expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
    });

    it('labels its state for screen readers', () => {
        const button = document.createElement('button');
        mountThemeToggle(button);

        expect(button.getAttribute('aria-label')).toContain('automatic');
        button.click();
        expect(button.getAttribute('aria-label')).toContain('light');
    });

    it('stores nothing besides the theme choice', () => {
        const button = document.createElement('button');
        mountThemeToggle(button);
        button.click();

        expect(Object.keys(localStorage)).toEqual([STORAGE_KEY]);
        expect(localStorage.getItem(STORAGE_KEY)).toBe('light');
    });
});
