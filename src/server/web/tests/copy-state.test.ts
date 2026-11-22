/**
 * src/pages/copy-state.ts: after a copy a button says "Copied" with a check
 * icon, tinted, for two seconds, a polite live region announces it, and then the
 * button has exactly its own content back - also when Lit renders it.
 */

import { html, render } from 'lit';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { COPIED_FOR_MS, CopyFeedback, createLiveRegion } from '../src/pages/copy-state';
import { icon } from '../src/pages/icons';
import { readCatalogue } from '../scripts/shells';
import { setPageLanguage } from './page-language';

/** What a screen reader and the eye get from `button`: name, visible text, icons, class. */
function state(button: HTMLButtonElement) {
    // Text and elements only: Lit's comment markers are no content.
    const visible = [...button.childNodes].filter(
        (node) =>
            node.nodeType === Node.TEXT_NODE ||
            (node instanceof Element && !node.closest('[hidden]')),
    );
    return {
        text: visible.map((node) => node.textContent).join(''),
        icons: [...button.querySelectorAll<SVGElement>('[data-icon]')]
            .filter((svg) => !svg.closest('[hidden]'))
            .map((svg) => svg.dataset.icon),
        copied: button.classList.contains('cb-copied'),
    };
}

function quietButton(): HTMLButtonElement {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'cb-quiet';
    const label = document.createElement('span');
    label.className = 'cb-bar-label';
    label.textContent = 'Copy';
    button.append(icon('copy'), label);
    return button;
}

describe('CopyFeedback', () => {
    let region: HTMLElement;
    let feedback: CopyFeedback;

    beforeEach(async () => {
        vi.useFakeTimers();
        await setPageLanguage('en');
        document.body.replaceChildren();
        region = createLiveRegion();
        document.body.append(region);
        feedback = new CopyFeedback(region);
    });

    afterEach(() => {
        vi.useRealTimers();
        vi.unstubAllGlobals();
    });

    it('creates a visually hidden polite live region', () => {
        expect(region.getAttribute('aria-live')).toBe('polite');
        expect(region.getAttribute('role')).toBe('status');
        expect(region.className).toBe('cb-sr-only');
        expect(region.textContent).toBe('');
    });

    it('shows "Copied" with a check icon for two seconds, then restores the same nodes', () => {
        const button = quietButton();
        document.body.append(button);
        const own = [...button.childNodes];

        feedback.show(button);
        expect(state(button)).toEqual({ text: 'Copied', icons: ['check'], copied: true });
        expect(region.textContent).toBe('Copied to clipboard');
        // The label of a bar action stays hidden on narrow screens.
        expect(button.querySelector(':scope > span:not([hidden])')!.className).toBe('cb-bar-label');

        vi.advanceTimersByTime(COPIED_FOR_MS - 1);
        expect(state(button).copied).toBe(true);

        vi.advanceTimersByTime(1);
        expect(state(button)).toEqual({ text: 'Copy', icons: ['copy'], copied: false });
        expect([...button.childNodes]).toEqual(own);
        expect(region.textContent).toBe('');
    });

    it('lasts two seconds', () => {
        expect(COPIED_FOR_MS).toBe(2000);
    });

    it('uses the catalogue of the page language', async () => {
        await setPageLanguage('de');
        const button = quietButton();
        feedback.show(button);
        expect(state(button).text).toBe(readCatalogue('de')['shell.copied']);
        expect(region.textContent).toBe(readCatalogue('de')['shell.copiedToClipboard']);
    });

    it('keeps a visible label visible and renames a button named by aria-label', () => {
        const button = document.createElement('button');
        button.className = 'cb-btn';
        button.setAttribute('aria-label', 'Copy link');
        button.append(icon('copy'), 'Copy link');

        feedback.show(button);
        expect(button.getAttribute('aria-label')).toBe('Copied');
        expect(button.querySelector(':scope > span:not([hidden])')!.className).toBe('');

        vi.advanceTimersByTime(COPIED_FOR_MS);
        expect(button.getAttribute('aria-label')).toBe('Copy link');
        expect(state(button)).toEqual({ text: 'Copy link', icons: ['copy'], copied: false });
    });

    it('restarts the two seconds on a second copy of the same button', () => {
        const button = quietButton();
        feedback.show(button);
        vi.advanceTimersByTime(1500);
        feedback.show(button);
        expect(button.querySelectorAll('[data-copy-parked]')).toHaveLength(1);

        vi.advanceTimersByTime(1500);
        expect(state(button).copied).toBe(true);
        vi.advanceTimersByTime(500);
        expect(state(button)).toEqual({ text: 'Copy', icons: ['copy'], copied: false });
    });

    it('restores the previous button when another one is copied', () => {
        const first = quietButton();
        const second = quietButton();
        feedback.show(first);
        feedback.show(second);
        expect(state(first)).toEqual({ text: 'Copy', icons: ['copy'], copied: false });
        expect(state(second).copied).toBe(true);
    });

    it('copies the exact text and shows the state only when the clipboard took it', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined);
        vi.stubGlobal('navigator', { clipboard: { writeText } });
        const button = quietButton();

        await expect(feedback.copy(button, 'line 1\n  line 2\n')).resolves.toBe(true);
        expect(writeText).toHaveBeenCalledWith('line 1\n  line 2\n');
        expect(state(button).copied).toBe(true);

        feedback.restore();
        writeText.mockRejectedValue(new Error('denied'));
        await expect(feedback.copy(button, 'x')).resolves.toBe(false);
        expect(state(button)).toEqual({ text: 'Copy', icons: ['copy'], copied: false });
        expect(region.textContent).toBe('');
    });

    it('lets Lit keep updating a button it renders', () => {
        const host = document.createElement('div');
        const view = (label: string) =>
            html`<button type="button" class="cb-quiet">
                ${icon('copy')}<span class="cb-bar-label">${label}</span>
            </button>`;
        render(view('Copy'), host);
        const button = host.querySelector('button')!;

        feedback.show(button);
        render(view('Kopieren'), host);
        expect(state(button).text.trim()).toBe('Copied');

        vi.advanceTimersByTime(COPIED_FOR_MS);
        expect(state(button).text.trim()).toBe('Kopieren');
        render(view('Copy again'), host);
        expect(state(button).text.trim()).toBe('Copy again');
        expect(button.querySelectorAll('[data-icon]')).toHaveLength(1);
    });
});
