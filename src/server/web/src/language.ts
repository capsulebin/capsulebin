/**
 * Interface language of the page: catalogue lookup, plural forms, number, size,
 * date and expiry formatting, and the language prefix of internal paths.
 *
 * Each page is served under a language prefix (`/de/p/…`) as a shell generated
 * for that language (scripts/shells.ts), so `<html lang>` is the page language.
 * The texts come from `virtual:catalogue` (scripts/catalogue.ts): the web build
 * runs once per language, and each build's scripts carry that language's texts
 * only - the ones the scripts use, the shell has the rest. There is no choice
 * between catalogues at runtime, no reloading and no translation of the static
 * shell.
 *
 * Catalogue values are plain text. Callers put them into the page as text
 * (`textContent`, Lit text bindings), never as HTML.
 *
 * Formatting follows the page language, not the browser locale, so that text and
 * numbers on a page never mix two languages.
 */

import { KNOWN_LANGUAGES, type Language, isKnownLanguage } from '@capsulebin/core';

import { DEFAULT_LANGUAGE } from './default-language';
import type en from './i18n/en.json';
import catalogue from 'virtual:catalogue';

export type { Language };

export type MessageKey = keyof typeof en;

/** A text without plural forms, for {@link t}. */
export type TextKey = Exclude<MessageKey, `${string}#${string}`>;

/**
 * A text with plural forms: the catalogue holds `<key>#<category>` for every
 * category of `Intl.PluralRules` in that language (`one`/`other` for en and de).
 */
export type PluralKey = {
    [K in MessageKey]: K extends `${infer Base}#other` ? Base : never;
}[MessageKey];

/** Where the language switch stores the choice. `public/language-init.js` reads it. */
export const LANGUAGE_STORAGE_KEY = 'capsulebin.language';

/**
 * The language of this page, from `<html lang>`. The shell and the scripts of a
 * page come from the same language's build, so this is the catalogue's language.
 */
export function pageLanguage(): Language {
    const primary = document.documentElement.lang.split('-')[0]?.toLowerCase() ?? '';
    return isKnownLanguage(primary) ? primary : DEFAULT_LANGUAGE;
}

/** Replaces `{name}` in `text` with the matching parameter. */
function interpolate(text: string, params: Readonly<Record<string, string | number>>): string {
    return text.replace(/\{(\w+)\}/g, (placeholder, name: string) => {
        const value = params[name];
        if (value === undefined) return placeholder;
        return typeof value === 'number' ? formatNumber(value) : value;
    });
}

/** The build has checked that the catalogue has every key the scripts name. */
function lookup(key: string): string {
    return catalogue[key] ?? key;
}

/** The text for `key` in the page language; numbers in `params` are formatted. */
export function t(key: TextKey, params: Readonly<Record<string, string | number>> = {}): string {
    return interpolate(lookup(key), params);
}

/** The plural form of `key` for `count`, with `{count}` and `params` filled in. */
export function tn(
    key: PluralKey,
    count: number,
    params: Readonly<Record<string, string | number>> = {},
): string {
    const category = new Intl.PluralRules(pageLanguage()).select(count);
    return interpolate(lookup(`${key}#${category}`), { ...params, count });
}

/** A number grouped the page language's way (`600,000` / `600.000`). */
export function formatNumber(value: number): string {
    return new Intl.NumberFormat(pageLanguage()).format(value);
}

/** A byte count as a short size (`1.5 KiB` / `1,5 KiB`). */
export function formatSize(bytes: number): string {
    if (bytes < 1024) return t('format.size.bytes', { size: bytes });

    const oneDecimal = new Intl.NumberFormat(pageLanguage(), {
        minimumFractionDigits: 1,
        maximumFractionDigits: 1,
    });
    if (bytes < 1024 * 1024) {
        return t('format.size.kib', { size: oneDecimal.format(bytes / 1024) });
    }
    return t('format.size.mib', { size: oneDecimal.format(bytes / (1024 * 1024)) });
}

/**
 * A Unix timestamp (seconds) as a short relative expiry, or "no expiry" for
 * `null`. Same thresholds as `formatExpiry` in `@capsulebin/core`.
 */
export function formatExpiry(
    expiresAt: number | null,
    nowSeconds: number = Math.floor(Date.now() / 1000),
): string {
    if (expiresAt === null) return t('expiry.none');

    const seconds = expiresAt - nowSeconds;
    if (seconds <= 0) return t('expiry.expired');

    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return tn('expiry.inMinutes', minutes);

    const hours = Math.round(minutes / 60);
    if (hours < 24) return tn('expiry.inHours', hours);

    return tn('expiry.inDays', Math.round(hours / 24));
}

/** Date and time in the page language. */
export function formatDateTime(date: Date): string {
    return new Intl.DateTimeFormat(pageLanguage(), {
        dateStyle: 'medium',
        timeStyle: 'short',
    }).format(date);
}

/** A local absolute path under the page's language prefix: `/report` → `/de/report`. */
export function localPath(path: string): string {
    return `/${pageLanguage()}${path.startsWith('/') ? path : `/${path}`}`;
}

/**
 * The languages this instance offers, from the `<meta name="capsulebin-languages">`
 * the shell generator puts into every shell. Without it every known language is
 * offered.
 */
export function offeredLanguages(): Language[] {
    const meta = document.querySelector<HTMLMetaElement>('meta[name="capsulebin-languages"]');
    if (!meta) return [...KNOWN_LANGUAGES];
    return meta.content.split(/\s+/).filter(isKnownLanguage);
}

/** The name of a language in that language ("English", "Deutsch"). */
export function endonym(language: Language): string {
    const name = new Intl.DisplayNames([language], { type: 'language' }).of(language) ?? language;
    return name.charAt(0).toLocaleUpperCase(language) + name.slice(1);
}

/**
 * Stores the choice for the next page loads. `localStorage` may be blocked; the
 * choice then holds for this navigation only.
 */
export function storeLanguage(language: Language): void {
    try {
        localStorage.setItem(LANGUAGE_STORAGE_KEY, language);
    } catch {
        // Without persistence the server and the browser language decide next time.
    }
}

/** The current location (path, query, fragment) under the prefix of `language`. */
export function pathInLanguage(language: Language, location: Location = window.location): string {
    const [, prefix = '', tail = ''] = /^\/([^/]+)(.*)$/.exec(location.pathname) ?? [];
    const rest = isKnownLanguage(prefix) ? tail : location.pathname;
    return `/${language}${rest}${location.search}${location.hash}`;
}

/**
 * What choosing a language in the switch does:
 *
 * - `navigate` stores the choice and loads the same page (query and fragment
 *   included) under the new prefix;
 * - `store` only stores it, for pages whose fragment has already been removed
 *   (an opened capsule, the management page): reloading them would lose the key.
 */
export type SwitchMode = 'navigate' | 'store';

export function chooseLanguage(
    language: Language,
    mode: SwitchMode,
    location: Location = window.location,
): void {
    storeLanguage(language);
    if (mode === 'navigate' && language !== pageLanguage()) {
        location.assign(pathInLanguage(language, location));
    }
}
