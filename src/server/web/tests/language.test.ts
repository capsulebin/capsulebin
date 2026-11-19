/**
 * src/language.ts and the language switch in the header: texts and formatting
 * follow `<html lang>`, never the browser locale, and a choice is stored and
 * applied under the new prefix.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
    LANGUAGE_STORAGE_KEY,
    chooseLanguage,
    endonym,
    formatDateTime,
    formatExpiry,
    formatNumber,
    formatSize,
    localPath,
    offeredLanguages,
    pageLanguage,
    pathInLanguage,
    t,
    tn,
} from '../src/language';
import { languageFromNextPage, mountLanguageSwitch } from '../src/pages/shell';

function setPage(lang: string, meta?: string): void {
    document.documentElement.lang = lang;
    document.head.replaceChildren();
    if (meta !== undefined) {
        const el = document.createElement('meta');
        el.name = 'capsulebin-languages';
        el.content = meta;
        el.dataset.default = 'en';
        document.head.append(el);
    }
}

function fakeLocation(path: string, search = '', hash = '') {
    return {
        pathname: path,
        search,
        hash,
        assign: vi.fn(),
        replace: vi.fn(),
    } as unknown as Location & {
        assign: ReturnType<typeof vi.fn>;
        replace: ReturnType<typeof vi.fn>;
    };
}

beforeEach(() => {
    localStorage.clear();
    setPage('en');
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('page language', () => {
    it('comes from <html lang>', () => {
        setPage('de');
        expect(pageLanguage()).toBe('de');
        setPage('de-AT');
        expect(pageLanguage()).toBe('de');
    });

    it('falls back to English for a missing or unknown lang', () => {
        setPage('');
        expect(pageLanguage()).toBe('en');
        setPage('fr');
        expect(pageLanguage()).toBe('en');
    });
});

describe('texts', () => {
    it('come from the catalogue of the page language', () => {
        expect(t('expiry.none')).toBe('No expiry');
        setPage('de');
        expect(t('expiry.none')).toBe('Kein Ablauf');
    });

    it('take plural forms by the language rules', () => {
        expect(tn('expiry.inMinutes', 1)).toBe('Expires in 1 minute');
        expect(tn('expiry.inMinutes', 2)).toBe('Expires in 2 minutes');
        setPage('de');
        expect(tn('expiry.inMinutes', 1)).toBe('Läuft in 1 Minute ab');
        expect(tn('expiry.inMinutes', 2)).toBe('Läuft in 2 Minuten ab');
    });

    it('fill in parameters, numbers formatted for the language', () => {
        setPage('de');
        expect(tn('expiry.inDays', 1500)).toBe('Läuft in 1.500 Tagen ab');
    });
});

describe('formatting', () => {
    /**
     * The browser says German, the page is English: the page wins. Node's default
     * Intl locale (the test environment's) plays no part either, as both
     * languages give their own result below.
     */
    it('ignores the browser locale', () => {
        vi.stubGlobal('navigator', { ...navigator, language: 'de-DE', languages: ['de-DE'] });
        expect(formatSize(1536)).toBe('1.5 KiB');
        expect(formatNumber(600000)).toBe('600,000');
    });

    it('formats sizes', () => {
        expect(formatSize(512)).toBe('512 B');
        expect(formatSize(1536)).toBe('1.5 KiB');
        expect(formatSize(5 * 1024 * 1024)).toBe('5.0 MiB');
        setPage('de');
        expect(formatSize(1536)).toBe('1,5 KiB');
        expect(formatSize(5 * 1024 * 1024)).toBe('5,0 MiB');
    });

    it('groups numbers', () => {
        expect(formatNumber(600000)).toBe('600,000');
        setPage('de');
        expect(formatNumber(600000)).toBe('600.000');
    });

    it('formats expiry with plural forms', () => {
        const now = 1_800_000_000;
        expect(formatExpiry(null, now)).toBe('No expiry');
        expect(formatExpiry(now, now)).toBe('Expired');
        expect(formatExpiry(now + 60, now)).toBe('Expires in 1 minute');
        expect(formatExpiry(now + 2 * 3600, now)).toBe('Expires in 2 hours');
        expect(formatExpiry(now + 86400, now)).toBe('Expires in 1 day');

        setPage('de');
        expect(formatExpiry(null, now)).toBe('Kein Ablauf');
        expect(formatExpiry(now - 1, now)).toBe('Abgelaufen');
        expect(formatExpiry(now + 60, now)).toBe('Läuft in 1 Minute ab');
        expect(formatExpiry(now + 120, now)).toBe('Läuft in 2 Minuten ab');
        expect(formatExpiry(now + 3600, now)).toBe('Läuft in 1 Stunde ab');
        expect(formatExpiry(now + 3 * 86400, now)).toBe('Läuft in 3 Tagen ab');
    });

    it('formats dates in the page language', () => {
        // Noon UTC is the same calendar day in every time zone the tests may run in.
        const date = new Date(Date.UTC(2026, 10, 19, 12, 0));
        expect(formatDateTime(date)).toContain('Nov 19, 2026');
        setPage('de');
        expect(formatDateTime(date)).toContain('19.11.2026');
    });
});

describe('paths', () => {
    it('put local paths under the page prefix', () => {
        expect(localPath('/report')).toBe('/en/report');
        setPage('de');
        expect(localPath('/report?id=x')).toBe('/de/report?id=x');
        expect(localPath('/')).toBe('/de/');
    });

    it('move the current location to another prefix, query and fragment included', () => {
        const location = fakeLocation('/en/p/abc', '?q=1', '#v1.key');
        expect(pathInLanguage('de', location)).toBe('/de/p/abc?q=1#v1.key');
        expect(pathInLanguage('de', fakeLocation('/en/'))).toBe('/de/');
        expect(pathInLanguage('en', fakeLocation('/report'))).toBe('/en/report');
    });
});

describe('offered languages', () => {
    it('come from the meta element, unknown codes dropped', () => {
        setPage('en', 'de en xx');
        expect(offeredLanguages()).toEqual(['de', 'en']);
    });

    it('are all known languages without the meta (dev server)', () => {
        expect(offeredLanguages()).toEqual(['en', 'de']);
    });

    it('are named in their own language', () => {
        expect(endonym('en')).toBe('English');
        expect(endonym('de')).toBe('Deutsch');
    });
});

describe('choosing a language', () => {
    it('stores the choice and loads the page under the new prefix', () => {
        const location = fakeLocation('/en/report', '?id=x', '#frag');
        chooseLanguage('de', 'navigate', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(location.replace).toHaveBeenCalledWith('/de/report?id=x#frag');
    });

    // An entry left behind would keep the key in its fragment after the page has
    // removed it from its own, and "Back" would bring it back.
    it('replaces the history entry instead of adding one', () => {
        const location = fakeLocation('/en/p/abc', '', '#v1.key');
        chooseLanguage('de', 'navigate', location);
        expect(location.replace).toHaveBeenCalledTimes(1);
        expect(location.assign).not.toHaveBeenCalled();
    });

    it('only stores in store mode', () => {
        const location = fakeLocation('/en/p/abc');
        chooseLanguage('de', 'store', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(location.replace).not.toHaveBeenCalled();
    });

    it('does not reload for the language the page already has', () => {
        const location = fakeLocation('/en/');
        chooseLanguage('en', 'navigate', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(location.replace).not.toHaveBeenCalled();
    });
});

describe('language switch', () => {
    function mount(meta?: string) {
        if (meta !== undefined) setPage(document.documentElement.lang, meta);
        const container = document.createElement('nav');
        container.hidden = true;
        const handle = mountLanguageSwitch(container);
        const buttons = [...container.querySelectorAll('button')];
        return { container, handle, buttons };
    }

    it('lists the offered languages by name and marks the current one', () => {
        setPage('de');
        const { container, buttons } = mount('en de');
        expect(container.hidden).toBe(false);
        expect(buttons.map((b) => [b.textContent, b.lang, b.getAttribute('aria-current')])).toEqual(
            [
                ['English', 'en', null],
                ['Deutsch', 'de', 'true'],
            ],
        );
    });

    it('stays hidden with a single language', () => {
        expect(mount('en').container.hidden).toBe(true);
    });

    it('calls back in store-only mode', () => {
        const { handle, buttons } = mount('en de');
        const stored = vi.fn();
        handle.storeOnly(stored);
        buttons[1]!.click();
        expect(stored).toHaveBeenCalledWith('de');
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
    });

    it('keeps a single notice when store-only mode is entered twice', () => {
        const { container, handle, buttons } = mount('en de');
        const parent = document.createElement('div');
        parent.append(container);
        const shell = { languageSwitch: handle };
        languageFromNextPage(shell);
        languageFromNextPage(shell);
        buttons[1]!.click();
        const notices = parent.querySelectorAll('[data-language-notice]');
        expect(notices).toHaveLength(1);
        expect(notices[0]!.textContent).toBe(t('shell.languageNextPage'));
    });
});
