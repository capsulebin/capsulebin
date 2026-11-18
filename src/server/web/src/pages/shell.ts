/**
 * Shared initialisation and DOM helpers for every page.
 *
 * Each page is its own document (multi-page application, no router) but they
 * share header, footer, theme and DOM utilities.
 */

import '../styles/tokens.css';
import '../styles/base.css';

import { initTheme, mountThemeToggle } from '../theme';

// --- boot --------------------------------------------------------------------

export function initShell(): void {
    // theme-init.js has already set the attribute before the paint; this call keeps
    // the state consistent in case that script was blocked.
    initTheme();

    const toggle = document.querySelector<HTMLButtonElement>('[data-theme-toggle]');
    if (toggle) mountThemeToggle(toggle);
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
