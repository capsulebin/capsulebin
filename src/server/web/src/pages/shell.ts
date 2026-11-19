/**
 * Shared initialisation and DOM helpers for every page.
 *
 * Each page is its own document (multi-page application, no router) but they
 * share header, footer, theme and DOM utilities.
 */

import '../styles/tokens.css';
import '../styles/base.css';

import {
    type Language,
    type SwitchMode,
    chooseLanguage,
    endonym,
    offeredLanguages,
    pageLanguage,
    t,
} from '../language';
import { initTheme, mountThemeToggle } from '../theme';

// --- boot --------------------------------------------------------------------

export interface Shell {
    /** The language switch in the header, if the page has one. */
    languageSwitch: LanguageSwitch | null;
}

export function initShell(): Shell {
    // theme-init.js has already set the attribute before the paint; this call keeps
    // the state consistent in case that script was blocked.
    initTheme();

    const toggle = document.querySelector<HTMLButtonElement>('[data-theme-toggle]');
    if (toggle) mountThemeToggle(toggle);

    const languages = document.querySelector<HTMLElement>('[data-language-switch]');
    return { languageSwitch: languages ? mountLanguageSwitch(languages) : null };
}

// --- language switch ---------------------------------------------------------

export interface LanguageSwitch {
    /**
     * From now on a choice is only stored, not applied by loading the page under
     * the new prefix; `onStored` runs after each choice (to tell the user it applies
     * from the next page on). For pages whose fragment is already gone.
     */
    storeOnly(onStored: (language: Language) => void): void;
    /**
     * The live region next to the switch for a notice about it, added empty on the
     * first call. It exists before its text changes, so the change is announced.
     */
    addNotice(): HTMLElement;
}

/**
 * Fills `container` with one button per offered language, named in that language,
 * the page language marked with `aria-current`. With a single language there is
 * nothing to choose and the switch stays hidden.
 */
export function mountLanguageSwitch(container: HTMLElement): LanguageSwitch {
    const current = pageLanguage();
    const offered = offeredLanguages();
    let mode: SwitchMode = 'navigate';
    let onStored: ((language: Language) => void) | null = null;
    let notice: HTMLElement | null = null;

    container.replaceChildren(
        ...offered.map((language) => {
            const button = document.createElement('button');
            button.type = 'button';
            button.lang = language;
            button.dataset.language = language;
            button.textContent = endonym(language);
            if (language === current) button.setAttribute('aria-current', 'true');
            button.addEventListener('click', () => {
                chooseLanguage(language, mode);
                onStored?.(language);
            });
            return button;
        }),
    );
    container.hidden = offered.length < 2;

    return {
        storeOnly(callback) {
            mode = 'store';
            onStored = callback;
        },
        addNotice() {
            if (notice) return notice;
            notice = document.createElement('span');
            notice.className = 'cb-status';
            notice.setAttribute('role', 'status');
            notice.dataset.languageNotice = '';
            container.before(notice);
            return notice;
        },
    };
}

/**
 * For pages that must not be reloaded any more - a retrieval or deletion has
 * started, or the key is gone from the address: from now on the switch only
 * stores the choice and says next to it that the choice applies from the next
 * page on. Calling it again changes nothing.
 */
export function languageFromNextPage(shell: Shell): void {
    const { languageSwitch } = shell;
    if (!languageSwitch) return;

    const notice = languageSwitch.addNotice();
    languageSwitch.storeOnly(() => {
        notice.textContent = t('shell.languageNextPage');
    });
}

// --- DOM helpers -------------------------------------------------------------

/** Clones a <template> into the element matching `rootAttr`, then fades it
 *  in via the `.cb-enter` CSS animation. */
export function showTemplate(id: string, rootAttr: string): void {
    const main = document.querySelector<HTMLElement>(`[${rootAttr}]`)!;
    const tpl = document.getElementById(id) as HTMLTemplateElement;
    main.replaceChildren(tpl.content.cloneNode(true));

    main.classList.add('cb-enter');
    main.addEventListener('animationend', () => main.classList.remove('cb-enter'), { once: true });
}

/** Sets the text content of the first element matching `[data-{attr}]`. */
export function setText(attr: string, value: string): void {
    const el = document.querySelector<HTMLElement>(`[data-${attr}]`);
    if (el) el.textContent = value;
}

/** Sets the value of the first input matching `[data-{attr}]`. */
export function setValue(attr: string, value: string): void {
    const el = document.querySelector<HTMLInputElement>(`[data-${attr}]`);
    if (el) el.value = value;
}

/** Returns the first element matching `[data-{attr}]`. */
export function byAttr<T extends HTMLElement>(attr: string): T {
    return document.querySelector<T>(`[data-${attr}]`)!;
}

/** Returns an element by id, cast to the expected type. */
export function byId<T extends HTMLElement>(id: string): T {
    return document.getElementById(id) as T;
}
