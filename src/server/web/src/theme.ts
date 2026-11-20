/**
 * Theme toggle.
 *
 * The Content Security Policy forbids inline scripts, so the usual
 * flash-of-unstyled-content snippet in `<head>` is not available. Instead
 * `public/theme-init.js` sets the attribute before the first paint; this module
 * only handles the interactive toggle.
 *
 * Only the preference itself is persisted. `localStorage` never sees the
 * fragment, a password, key material or plaintext - that holds for the whole
 * application, not just this module.
 */

import { type TextKey, t } from './language';

const STORAGE_KEY = 'capsulebin.theme';

export type ThemeChoice = 'light' | 'dark' | 'auto';

const CHOICES: readonly ThemeChoice[] = ['light', 'dark', 'auto'];

function isThemeChoice(value: unknown): value is ThemeChoice {
    return typeof value === 'string' && (CHOICES as readonly string[]).includes(value);
}

/**
 * Read the stored choice. `localStorage` can throw, for instance when cookies
 * and site data are blocked. That is not an error case; it simply means there is
 * no preference, so `auto` applies.
 */
export function readChoice(): ThemeChoice {
    try {
        const stored = localStorage.getItem(STORAGE_KEY);
        return isThemeChoice(stored) ? stored : 'auto';
    } catch {
        return 'auto';
    }
}

function writeChoice(choice: ThemeChoice): void {
    try {
        if (choice === 'auto') {
            localStorage.removeItem(STORAGE_KEY);
        } else {
            localStorage.setItem(STORAGE_KEY, choice);
        }
    } catch {
        // Without persistence the choice applies to this tab only. No reason to fail.
    }
}

/**
 * Sets or removes `data-theme` on the root element. For `auto` the attribute is
 * removed so that the media query in tokens.css takes over again.
 *
 * `<meta name="color-scheme">` (set by public/theme-init.js) is left alone: once
 * the stylesheet applies, `color-scheme` in tokens.css overrides it, and the next
 * document runs theme-init.js again with the new choice.
 */
export function applyChoice(choice: ThemeChoice): void {
    const root = document.documentElement;
    if (choice === 'auto') {
        delete root.dataset.theme;
    } else {
        root.dataset.theme = choice;
    }
}

/** Which theme is actually visible right now. */
export function effectiveTheme(choice: ThemeChoice = readChoice()): 'light' | 'dark' {
    if (choice !== 'auto') return choice;
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/**
 * Call before the first paint so that no theme change flashes up.
 */
export function initTheme(): void {
    applyChoice(readChoice());
}

const LABEL: Record<ThemeChoice, TextKey> = {
    light: 'theme.light',
    dark: 'theme.dark',
    auto: 'theme.auto',
};

/**
 * Wires up a toggle. The order is auto -> light -> dark -> auto, so that the
 * system preference stays reachable instead of being lost after the first click.
 */
export function mountThemeToggle(button: HTMLButtonElement): void {
    const next: Record<ThemeChoice, ThemeChoice> = {
        auto: 'light',
        light: 'dark',
        dark: 'auto',
    };

    const render = (choice: ThemeChoice): void => {
        button.dataset.theme = choice;
        button.setAttribute('aria-label', t(LABEL[choice]));
        button.textContent = choice === 'auto' ? '◐' : choice === 'light' ? '☀' : '☾';
    };

    button.addEventListener('click', () => {
        const choice = next[readChoice()];
        writeChoice(choice);
        applyChoice(choice);
        render(choice);
    });

    render(readChoice());
}
