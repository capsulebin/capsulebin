/**
 * Tests for the theme toggle.
 *
 * Most important point: `localStorage` may only ever see the theme choice.
 * Anything else - fragment, password, key material, plaintext - would defeat the
 * zero-knowledge property through a persistence side channel.
 */

import { beforeEach, describe, expect, it } from 'vitest';

import { applyChoice, effectiveTheme, mountThemeToggle, readChoice } from '../src/theme';
import { setPageLanguage } from './page-language';

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
    beforeEach(async () => {
        localStorage.clear();
        delete document.documentElement.dataset.theme;
        fakeMatchMedia(false);
        await setPageLanguage('en');
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

    /** The name stays the action; the state is the description (`title`) and the icon. */
    it('keeps its name and describes its state', () => {
        const button = document.createElement('button');
        mountThemeToggle(button);

        expect(button.getAttribute('aria-label')).toBe('Switch theme');
        expect(button.title).toContain('automatic');
        button.click();
        expect(button.getAttribute('aria-label')).toBe('Switch theme');
        expect(button.title).toContain('light');
    });

    /** A glyph like U+25D0 depends on the installed fonts and rendered as a speck. */
    it('shows an inline SVG icon per state and no text', () => {
        const button = document.createElement('button');
        button.textContent = 'old content';
        mountThemeToggle(button);

        const icons = (): string[] =>
            [...button.children].map((child) => (child as SVGElement).dataset.icon ?? '');
        const only = (): SVGSVGElement => {
            expect(button.children).toHaveLength(1);
            const svg = button.firstElementChild as SVGSVGElement;
            expect(svg.namespaceURI).toBe('http://www.w3.org/2000/svg');
            expect(svg.getAttribute('aria-hidden')).toBe('true');
            expect(button.textContent!.trim()).toBe('');
            return svg;
        };

        only();
        expect(icons()).toEqual(['theme']);
        button.click();
        only();
        expect(icons()).toEqual(['sun']);
        button.click();
        expect(icons()).toEqual(['moon']);
        button.click();
        expect(icons()).toEqual(['theme']);
    });

    it('stores nothing besides the theme choice', () => {
        const button = document.createElement('button');
        mountThemeToggle(button);
        button.click();

        expect(Object.keys(localStorage)).toEqual([STORAGE_KEY]);
        expect(localStorage.getItem(STORAGE_KEY)).toBe('light');
    });
});
