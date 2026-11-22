/**
 * The "Copied" state of a copy button, shared by the view page (Lit) and the
 * create page (plain DOM).
 *
 * After a successful copy the button shows a check icon and the catalogue's
 * "Copied" for {@link COPIED_FOR_MS}, tinted by `.cb-copied`, and a polite live
 * region says that the text is on the clipboard. Then the button gets back
 * exactly what it had: its own child nodes are not rebuilt but parked in a
 * hidden element inside the button and moved back. They stay the same nodes,
 * so a Lit template that owns them keeps updating them meanwhile, and nothing
 * the page attached to them is lost.
 *
 * One {@link CopyFeedback} per page: a second copy restores the previous button
 * first, so at most one button is in the copied state.
 */

import { t } from '../language';
import { icon } from './icons';

/** How long a button shows "Copied". */
export const COPIED_FOR_MS = 2000;

/** The class of a bar action label that narrow screens hide (base.css). */
const BAR_LABEL = 'cb-bar-label';

interface Copied {
    button: HTMLButtonElement;
    /** The button's own children while it shows "Copied". */
    parked: HTMLElement;
    /** The button's `aria-label` before, if it had one. */
    ariaLabel: string | null;
    timer: ReturnType<typeof setTimeout>;
}

/**
 * A polite live region for the announcement, visually hidden. It has to be in
 * the document before its text changes, so the page adds it when it renders.
 */
export function createLiveRegion(): HTMLElement {
    const region = document.createElement('span');
    region.className = 'cb-sr-only';
    region.setAttribute('role', 'status');
    region.setAttribute('aria-live', 'polite');
    return region;
}

export class CopyFeedback {
    readonly #region: HTMLElement;
    #copied: Copied | null = null;

    /** `region`: the page's live region, e.g. from {@link createLiveRegion}. */
    constructor(region: HTMLElement) {
        this.#region = region;
    }

    /**
     * Writes `text` to the clipboard and, if that worked, shows the copied state
     * on `button`. Resolves to whether the text was copied; a refused clipboard
     * leaves the button unchanged.
     */
    async copy(button: HTMLButtonElement, text: string): Promise<boolean> {
        try {
            await navigator.clipboard.writeText(text);
        } catch {
            return false;
        }
        this.show(button);
        return true;
    }

    /** Shows the copied state on `button` (restarting it if it is already shown). */
    show(button: HTMLButtonElement): void {
        this.restore();

        const parked = document.createElement('span');
        parked.hidden = true;
        parked.dataset.copyParked = '';
        parked.append(...button.childNodes);

        const label = document.createElement('span');
        label.textContent = t('shell.copied');
        // A label that narrow screens hide stays hidden there in the copied state too.
        if (parked.querySelector(`.${BAR_LABEL}`)) label.className = BAR_LABEL;

        button.append(parked, icon('check'), label);
        const ariaLabel = button.getAttribute('aria-label');
        if (ariaLabel !== null) button.setAttribute('aria-label', t('shell.copied'));
        button.classList.add('cb-copied');
        this.#region.textContent = t('shell.copiedToClipboard');

        this.#copied = {
            button,
            parked,
            ariaLabel,
            timer: setTimeout(() => this.restore(), COPIED_FOR_MS),
        };
    }

    /** Gives the button in the copied state its own content back; nothing if there is none. */
    restore(): void {
        const copied = this.#copied;
        if (!copied) return;
        this.#copied = null;
        clearTimeout(copied.timer);

        const { button, parked, ariaLabel } = copied;
        button.replaceChildren(...parked.childNodes);
        if (ariaLabel !== null) button.setAttribute('aria-label', ariaLabel);
        button.classList.remove('cb-copied');
        this.#region.textContent = '';
    }
}
