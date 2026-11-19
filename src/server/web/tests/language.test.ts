/**
 * src/language.ts and the language switch in the header: texts and formatting
 * follow `<html lang>`, never the browser locale, and a choice is stored and
 * applied on the path of the new language: without a prefix for the instance's
 * default language, under `/<lang>` for every other.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
    LANGUAGE_STORAGE_KEY,
    chooseLanguage,
    defaultLanguage,
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
    type Language,
} from '../src/language';
import { LANGUAGE_NEXT_PAGE } from '../src/i18n/language-next-page';
import { isLeavingPage, languageFromNextPage, mountLanguageSwitch } from '../src/pages/shell';
import { setPageLanguage } from './page-language';

/** `meta` is the content of the languages meta, `fallback` its `data-default`. */
async function setPage(lang: Language, meta?: string, fallback = 'en'): Promise<void> {
    await setPageLanguage(lang);
    document.head.replaceChildren();
    if (meta !== undefined) {
        const el = document.createElement('meta');
        el.name = 'capsulebin-languages';
        el.content = meta;
        el.dataset.default = fallback;
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

beforeEach(async () => {
    localStorage.clear();
    await setPage('en');
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('page language', () => {
    it('comes from <html lang>', () => {
        document.documentElement.lang = 'de';
        expect(pageLanguage()).toBe('de');
        document.documentElement.lang = 'de-AT';
        expect(pageLanguage()).toBe('de');
    });

    it('falls back to English for a missing or unknown lang', () => {
        document.documentElement.lang = '';
        expect(pageLanguage()).toBe('en');
        document.documentElement.lang = 'fr';
        expect(pageLanguage()).toBe('en');
    });
});

describe('texts', () => {
    it('come from the catalogue of the page language', async () => {
        expect(t('expiry.none')).toBe('No expiry');
        await setPage('de');
        expect(t('expiry.none')).toBe('Kein Ablauf');
    });

    it('take plural forms by the language rules', async () => {
        expect(tn('expiry.inMinutes', 1)).toBe('Expires in 1 minute');
        expect(tn('expiry.inMinutes', 2)).toBe('Expires in 2 minutes');
        await setPage('de');
        expect(tn('expiry.inMinutes', 1)).toBe('Läuft in 1 Minute ab');
        expect(tn('expiry.inMinutes', 2)).toBe('Läuft in 2 Minuten ab');
    });

    it('fill in parameters, numbers formatted for the language', async () => {
        await setPage('de');
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

    it('formats sizes', async () => {
        expect(formatSize(512)).toBe('512 B');
        expect(formatSize(1536)).toBe('1.5 KiB');
        expect(formatSize(5 * 1024 * 1024)).toBe('5.0 MiB');
        await setPage('de');
        expect(formatSize(1536)).toBe('1,5 KiB');
        expect(formatSize(5 * 1024 * 1024)).toBe('5,0 MiB');
    });

    it('groups numbers', async () => {
        expect(formatNumber(600000)).toBe('600,000');
        await setPage('de');
        expect(formatNumber(600000)).toBe('600.000');
    });

    it('formats expiry with plural forms', async () => {
        const now = 1_800_000_000;
        expect(formatExpiry(null, now)).toBe('No expiry');
        expect(formatExpiry(now, now)).toBe('Expired');
        expect(formatExpiry(now + 60, now)).toBe('Expires in 1 minute');
        expect(formatExpiry(now + 2 * 3600, now)).toBe('Expires in 2 hours');
        expect(formatExpiry(now + 86400, now)).toBe('Expires in 1 day');

        await setPage('de');
        expect(formatExpiry(null, now)).toBe('Kein Ablauf');
        expect(formatExpiry(now - 1, now)).toBe('Abgelaufen');
        expect(formatExpiry(now + 60, now)).toBe('Läuft in 1 Minute ab');
        expect(formatExpiry(now + 120, now)).toBe('Läuft in 2 Minuten ab');
        expect(formatExpiry(now + 3600, now)).toBe('Läuft in 1 Stunde ab');
        expect(formatExpiry(now + 3 * 86400, now)).toBe('Läuft in 3 Tagen ab');
    });

    it('formats dates in the page language', async () => {
        // Noon UTC is the same calendar day in every time zone the tests may run in.
        const date = new Date(Date.UTC(2026, 10, 19, 12, 0));
        expect(formatDateTime(date)).toContain('Nov 19, 2026');
        await setPage('de');
        expect(formatDateTime(date)).toContain('19.11.2026');
    });
});

describe('default language', () => {
    it('comes from data-default of the languages meta', async () => {
        await setPage('en', 'en de', 'de');
        expect(defaultLanguage()).toBe('de');
    });

    it.each([
        ['an unfilled server token', '%%capsulebin:default%%'],
        ['an unknown code', 'fr'],
        ['an empty value', ''],
    ])('falls back to en for %s', async (_, value) => {
        await setPage('de', 'en de', value);
        expect(defaultLanguage()).toBe('en');
    });

    it('falls back to en without the meta element', () => {
        expect(defaultLanguage()).toBe('en');
    });
});

describe('paths', () => {
    it('leave local paths without a prefix on a page in the default language', async () => {
        expect(localPath('/report')).toBe('/report');
        expect(localPath('/')).toBe('/');
        expect(localPath('report')).toBe('/report');
        await setPage('de', 'en de', 'de');
        expect(localPath('/report?id=x')).toBe('/report?id=x');
    });

    it('put local paths under the prefix of any other page language', async () => {
        await setPage('de');
        expect(localPath('/report?id=x')).toBe('/de/report?id=x');
        expect(localPath('/')).toBe('/de/');
        await setPage('en', 'en de', 'de');
        expect(localPath('/report')).toBe('/en/report');
    });

    it('move the current location to another language, query and fragment included', () => {
        const location = fakeLocation('/p/abc', '?q=1', '#v1.key');
        expect(pathInLanguage('de', location)).toBe('/de/p/abc?q=1#v1.key');
        expect(pathInLanguage('de', fakeLocation('/'))).toBe('/de/');
        expect(pathInLanguage('de', fakeLocation('/de/report'))).toBe('/de/report');
    });

    it('drop the prefix for the default language', () => {
        const location = fakeLocation('/de/p/abc', '?q=1', '#v1.key');
        expect(pathInLanguage('en', location)).toBe('/p/abc?q=1#v1.key');
        expect(pathInLanguage('en', fakeLocation('/de/'))).toBe('/');
        expect(pathInLanguage('en', fakeLocation('/de'))).toBe('/');
        expect(pathInLanguage('en', fakeLocation('/report'))).toBe('/report');
    });

    it('follow the configured default language', async () => {
        await setPage('de', 'en de', 'de');
        expect(pathInLanguage('en', fakeLocation('/p/abc', '', '#v1.key'))).toBe(
            '/en/p/abc#v1.key',
        );
        expect(pathInLanguage('de', fakeLocation('/en/report', '?id=x'))).toBe('/report?id=x');
    });

    // Only a known language is a prefix: `/p/…` and `/manage/…` are pages.
    it('keep a first segment that is no language', () => {
        expect(pathInLanguage('de', fakeLocation('/manage/abc'))).toBe('/de/manage/abc');
        expect(pathInLanguage('en', fakeLocation('/fr/report'))).toBe('/fr/report');
    });
});

describe('offered languages', () => {
    it('come from the meta element, unknown codes dropped', async () => {
        await setPage('en', 'de en xx');
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
        const location = fakeLocation('/report', '?id=x', '#frag');
        chooseLanguage('de', 'navigate', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(location.replace).toHaveBeenCalledWith('/de/report?id=x#frag');
    });

    it('loads the prefix-less page for the default language', async () => {
        await setPage('de');
        const location = fakeLocation('/de/report', '?id=x', '#frag');
        chooseLanguage('en', 'navigate', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(location.replace).toHaveBeenCalledWith('/report?id=x#frag');
    });

    // An entry left behind would keep the key in its fragment after the page has
    // removed it from its own, and "Back" would bring it back.
    it('replaces the history entry instead of adding one', () => {
        const location = fakeLocation('/p/abc', '', '#v1.key');
        chooseLanguage('de', 'navigate', location);
        expect(location.replace).toHaveBeenCalledTimes(1);
        expect(location.assign).not.toHaveBeenCalled();
    });

    it('only stores in store mode', () => {
        const location = fakeLocation('/p/abc');
        chooseLanguage('de', 'store', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
        expect(location.replace).not.toHaveBeenCalled();
    });

    it('does not reload for the language the page already has', () => {
        const location = fakeLocation('/');
        chooseLanguage('en', 'navigate', location);
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('en');
        expect(location.replace).not.toHaveBeenCalled();
    });

    it('says whether the page is being left', () => {
        expect(chooseLanguage('de', 'navigate', fakeLocation('/'))).toBe(true);
        expect(chooseLanguage('de', 'store', fakeLocation('/'))).toBe(false);
        expect(chooseLanguage('en', 'navigate', fakeLocation('/'))).toBe(false);
    });
});

describe('language switch', () => {
    async function mount(meta?: string) {
        if (meta !== undefined) await setPage(pageLanguage(), meta);
        const container = document.createElement('nav');
        container.hidden = true;
        const handle = mountLanguageSwitch(container);
        const buttons = [...container.querySelectorAll('button')];
        return { container, handle, buttons };
    }

    it('lists the offered languages by name and marks the current one', async () => {
        await setPage('de');
        const { container, buttons } = await mount('en de');
        expect(container.hidden).toBe(false);
        expect(buttons.map((b) => [b.textContent, b.lang, b.getAttribute('aria-current')])).toEqual(
            [
                ['English', 'en', null],
                ['Deutsch', 'de', 'true'],
            ],
        );
    });

    it('stays hidden with a single language', async () => {
        expect((await mount('en')).container.hidden).toBe(true);
    });

    it('calls back in store-only mode', async () => {
        const { handle, buttons } = await mount('en de');
        const stored = vi.fn();
        handle.storeOnly(stored);
        buttons[1]!.click();
        expect(stored).toHaveBeenCalledWith('de');
        expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe('de');
    });

    it('keeps a single notice when store-only mode is entered twice', async () => {
        const { container, handle, buttons } = await mount('en de');
        const parent = document.createElement('div');
        parent.append(container);
        const shell = { languageSwitch: handle };
        languageFromNextPage(shell);
        languageFromNextPage(shell);
        buttons[1]!.click();
        const notices = parent.querySelectorAll('[data-language-notice]');
        expect(notices).toHaveLength(1);
        expect(notices[0]!.textContent).toBe(LANGUAGE_NEXT_PAGE.de);
    });

    // AC27: whoever picks a language may not read the page's.
    it.each([
        ['en', 'de', 'Die Sprache gilt ab der nächsten Seite.'],
        ['de', 'en', 'The language applies from the next page on.'],
    ] as const)(
        'on a %s page shows the notice for %s in that language',
        async (page, chosen, text) => {
            await setPage(page);
            const { container, handle, buttons } = await mount('en de');
            const parent = document.createElement('div');
            parent.append(container);
            languageFromNextPage({ languageSwitch: handle });
            buttons.find((button) => button.lang === chosen)!.click();
            const notice = parent.querySelector<HTMLElement>('[data-language-notice]')!;
            expect(notice.textContent).toBe(text);
            expect(notice.lang).toBe(chosen);
        },
    );

    // The old document stays on screen until the new one arrives.
    it('is leaving and disables the page controls once a choice loads the page', async () => {
        const main = document.createElement('main');
        main.innerHTML = '<button></button><input /><select></select><textarea></textarea>';
        document.body.append(main);
        const { handle, buttons } = await mount('en de');
        const controls = [...main.children] as HTMLButtonElement[];

        // The page's own language: nothing is loaded.
        buttons[0]!.click();
        expect(handle.isLeaving()).toBe(false);
        expect(isLeavingPage({ languageSwitch: handle })).toBe(false);
        expect(controls.map((control) => control.disabled)).toEqual([false, false, false, false]);

        buttons[1]!.click();
        expect(handle.isLeaving()).toBe(true);
        expect(isLeavingPage({ languageSwitch: handle })).toBe(true);
        expect(controls.map((control) => control.disabled)).toEqual([true, true, true, true]);
        // The switch itself stays usable.
        expect(buttons.map((button) => button.disabled)).toEqual([false, false]);
        main.remove();
    });

    it('is not leaving in store-only mode', async () => {
        const { handle, buttons } = await mount('en de');
        languageFromNextPage({ languageSwitch: handle });
        buttons[1]!.click();
        expect(handle.isLeaving()).toBe(false);
    });

    it('is not leaving on a page without a switch', () => {
        expect(isLeavingPage({ languageSwitch: null })).toBe(false);
    });

    it('follows each further choice with the notice', async () => {
        const { container, handle, buttons } = await mount('en de');
        const parent = document.createElement('div');
        parent.append(container);
        languageFromNextPage({ languageSwitch: handle });
        buttons[1]!.click();
        buttons[0]!.click();
        const notice = parent.querySelector<HTMLElement>('[data-language-notice]')!;
        expect(notice.textContent).toBe(LANGUAGE_NEXT_PAGE.en);
        expect(notice.lang).toBe('en');
    });
});
