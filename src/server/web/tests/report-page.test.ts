/**
 * The report page wired to its real HTML: what is sent, that the key in a
 * pasted link never is, and that a refusal is explained from the catalogue by
 * its problem `code`, never with the server's (English) `detail`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import de from '../src/i18n/de.json';
import en from '../src/i18n/en.json';
import { LANGUAGE_NEXT_PAGE } from '../src/i18n/language-next-page';
import { LANGUAGE_STORAGE_KEY } from '../src/language';
import { servedShell, setPageLanguage } from './page-language';

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const KEY = 'v1.c2VjcmV0LWtleS1ub3QtdG8tYmUtc2VudA';

const CATALOGUES = { en, de };

interface Sent {
    url: string;
    method: string;
    body: string;
}

let sent: Sent[];

function stubServer(
    policy: { enabled: boolean; contact: string | null },
    reportStatus = 202,
    reportBody: object = { status: 'received' },
) {
    sent = [];
    vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
        sent.push({ url, method: init?.method ?? 'GET', body: String(init?.body ?? '') });
        if ((init?.method ?? 'GET') === 'GET') {
            return new Response(JSON.stringify(policy), { status: 200 });
        }
        return new Response(JSON.stringify(reportBody), { status: reportStatus });
    });
}

async function loadPage(search = '', lang: 'en' | 'de' = 'en'): Promise<void> {
    const page = new DOMParser().parseFromString(servedShell('report', lang), 'text/html');
    document.documentElement.innerHTML = page.documentElement.innerHTML;
    window.history.replaceState(null, '', `/${lang}/report${search}`);
    vi.resetModules();
    await setPageLanguage(lang);
    await import('../src/pages/report');
    await flush();
}

async function flush(): Promise<void> {
    for (let i = 0; i < 5; i++) await new Promise((resolve) => setTimeout(resolve, 0));
}

function field<T extends HTMLElement>(attr: string): T {
    return document.querySelector<T>(`[data-${attr}]`)!;
}

async function submit(): Promise<void> {
    field<HTMLFormElement>('report-form').dispatchEvent(
        new Event('submit', { cancelable: true, bubbles: true }),
    );
    await flush();
}

beforeEach(() => {
    stubServer({ enabled: true, contact: null });
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe('report page', () => {
    it('prefills the id from ?id= and shows the form', async () => {
        await loadPage(`?id=${ID}`);
        expect(field<HTMLInputElement>('report-paste').value).toBe(ID);
        expect(field('report-contact-note').hidden).toBe(true);
    });

    it('sends only the id from a pasted link, never the key', async () => {
        await loadPage();
        field<HTMLInputElement>('report-paste').value = `https://paste.example/p/${ID}#${KEY}`;
        field<HTMLTextAreaElement>('report-description').value = 'phishing page';
        field<HTMLSelectElement>('report-reason').value = 'phishing';
        await submit();

        const post = sent.find((s) => s.method === 'POST')!;
        expect(post.url).toBe('/api/v1/reports');
        expect(JSON.parse(post.body)).toEqual({
            paste_id: ID,
            reason: 'phishing',
            description: 'phishing page',
        });
        for (const request of sent) expect(request.body + request.url).not.toContain(KEY);
        // The field no longer holds the key either.
        expect(document.body.innerHTML).not.toContain(KEY);
        expect(document.querySelector('h1')!.textContent).toBe('Report received');
    });

    it('does not send without a usable id or description', async () => {
        await loadPage();
        field<HTMLInputElement>('report-paste').value = 'https://paste.example/p/short';
        field<HTMLTextAreaElement>('report-description').value = 'x';
        await submit();
        expect(field('report-error').hidden).toBe(false);

        field<HTMLInputElement>('report-paste').value = ID;
        field<HTMLTextAreaElement>('report-description').value = '   ';
        await submit();
        expect(sent.filter((s) => s.method === 'POST')).toHaveLength(0);
    });

    it('explains a rate limit', async () => {
        stubServer({ enabled: true, contact: null }, 429);
        await loadPage(`?id=${ID}`);
        field<HTMLTextAreaElement>('report-description').value = 'spam';
        await submit();
        expect(field('report-error').textContent).toMatch(/too many reports/i);
    });

    // Like a draft on the create page; the prefilled id alone is no started report.
    it.each(['report-paste', 'report-reason', 'report-description', 'report-contact'])(
        'keeps a started report on a language choice: first edit of %s',
        async (attr) => {
            localStorage.clear();
            sessionStorage.clear();
            await loadPage(`?id=${ID}`, 'de');
            expect(document.querySelector('[data-language-notice]')).toBeNull();

            field<HTMLTextAreaElement>('report-description').value = 'phishing page';
            field(attr).dispatchEvent(new Event('input', { bubbles: true }));
            document
                .querySelector<HTMLButtonElement>('[data-language-switch] [lang="en"]')!
                .click();

            expect(window.location.pathname + window.location.search).toBe(`/de/report?id=${ID}`);
            expect(field<HTMLTextAreaElement>('report-description').value).toBe('phishing page');
            expect(field<HTMLTextAreaElement>('report-description').disabled).toBe(false);
            const notice = document.querySelector('[data-language-notice]')!;
            expect(notice.textContent).toBe(LANGUAGE_NEXT_PAGE.en);
            expect(notice.getAttribute('lang')).toBe('en');
            // Only the choice is stored, nothing of the report.
            expect(Object.keys(localStorage)).toEqual([LANGUAGE_STORAGE_KEY]);
            expect(sessionStorage).toHaveLength(0);
        },
    );

    it('shows the contact address and the disabled state', async () => {
        stubServer({ enabled: true, contact: 'abuse@paste.example' });
        await loadPage();
        expect(field('report-contact-note').hidden).toBe(false);
        expect(field('report-contact-address').textContent).toBe('abuse@paste.example');

        stubServer({ enabled: false, contact: null });
        await loadPage();
        expect(document.querySelector('h1')!.textContent).toBe('Reports are not accepted here');
        expect(document.querySelector('[data-report-form]')).toBeNull();
    });
});

describe('report page errors', () => {
    const DETAIL = 'detail from the server';

    /** Every code the report endpoint (and the limits in front of it) answers with. */
    const CODES = [
        { status: 400, code: 'malformed_request', key: 'report.error.rejected' },
        { status: 400, code: 'invalid_id', key: 'report.error.invalidId' },
        { status: 413, code: 'payload_too_large', key: 'report.error.tooLarge' },
        { status: 429, code: 'rate_limited', key: 'report.error.rateLimited' },
        { status: 500, code: 'internal_error', key: 'report.error.server' },
        { status: 400, code: 'something_new', key: 'report.error.generic' },
    ] as const;

    async function refuse(lang: 'en' | 'de', status: number, body: object): Promise<string> {
        stubServer({ enabled: true, contact: null }, status, body);
        await loadPage(`?id=${ID}`, lang);
        field<HTMLTextAreaElement>('report-description').value = 'spam';
        await submit();
        expect(field('report-error').hidden).toBe(false);
        return field('report-error').textContent!;
    }

    for (const lang of ['en', 'de'] as const) {
        for (const { status, code, key } of CODES) {
            it(`explains ${code} in ${lang} from the catalogue, never with the detail`, async () => {
                const text = await refuse(lang, status, { status, code, detail: DETAIL });
                expect(text).toBe(CATALOGUES[lang][key]);
                expect(document.body.textContent).not.toContain(DETAIL);
            });
        }
    }

    it('takes a 429 without a code for a rate limit', async () => {
        expect(await refuse('de', 429, {})).toBe(de['report.error.rateLimited']);
    });

    it('falls back to the general text without a problem body', async () => {
        expect(await refuse('en', 502, {})).toBe(en['report.error.generic']);
    });

    it('shows the disabled state for reports_disabled', async () => {
        stubServer({ enabled: true, contact: null }, 403, { code: 'reports_disabled' });
        await loadPage(`?id=${ID}`, 'de');
        field<HTMLTextAreaElement>('report-description').value = 'spam';
        await submit();
        expect(document.querySelector('h1')!.textContent).toBe(de['report.disabled']);
    });

    it('explains a network failure', async () => {
        await loadPage(`?id=${ID}`, 'de');
        vi.stubGlobal('fetch', async () => {
            throw new TypeError('offline');
        });
        field<HTMLTextAreaElement>('report-description').value = 'spam';
        await submit();
        expect(field('report-error').textContent).toBe(de['report.error.network']);
    });

    it('checks the form in the page language', async () => {
        await loadPage('', 'de');
        await submit();
        expect(field('report-error').textContent).toBe(de['report.error.noId']);
        field<HTMLInputElement>('report-paste').value = ID;
        await submit();
        expect(field('report-error').textContent).toBe(de['report.error.noDescription']);
    });
});

describe('report page refuses a key in the free text', () => {
    const SHARE = `https://paste.example/p/${ID}#${KEY}`;
    const MANAGE = `https://paste.example/de/manage/${ID}#${KEY}`;

    function posts(): Sent[] {
        return sent.filter((s) => s.method === 'POST');
    }

    for (const lang of ['en', 'de'] as const) {
        it(`refuses a share link in the description, in ${lang}`, async () => {
            await loadPage(`?id=${ID}`, lang);
            const description = `This one: ${SHARE}`;
            field<HTMLTextAreaElement>('report-description').value = description;
            await submit();

            expect(posts()).toHaveLength(0);
            expect(field('report-error').hidden).toBe(false);
            expect(field('report-error').textContent).toBe(
                CATALOGUES[lang]['report.error.containsKey'],
            );
            expect(document.activeElement).toBe(field('report-description'));
            expect(field<HTMLTextAreaElement>('report-description').value).toBe(description);
        });

        it(`refuses a management link in the contact, in ${lang}`, async () => {
            await loadPage(`?id=${ID}`, lang);
            field<HTMLTextAreaElement>('report-description').value = 'spam';
            field<HTMLInputElement>('report-contact').value = MANAGE;
            await submit();

            expect(posts()).toHaveLength(0);
            expect(field('report-error').textContent).toBe(
                CATALOGUES[lang]['report.error.containsKey'],
            );
            expect(document.activeElement).toBe(field('report-contact'));
            expect(field<HTMLInputElement>('report-contact').value).toBe(MANAGE);
            expect(field<HTMLTextAreaElement>('report-description').value).toBe('spam');
        });
    }

    it('focuses the description when both fields contain a key', async () => {
        await loadPage(`?id=${ID}`);
        field<HTMLTextAreaElement>('report-description').value = SHARE;
        field<HTMLInputElement>('report-contact').value = MANAGE;
        await submit();

        expect(posts()).toHaveLength(0);
        expect(document.activeElement).toBe(field('report-description'));
        expect(field('report-error').textContent).toBe(en['report.error.containsKey']);
    });

    it('sends the report once the key is removed', async () => {
        await loadPage(`?id=${ID}`);
        field<HTMLTextAreaElement>('report-description').value = SHARE;
        await submit();
        expect(posts()).toHaveLength(0);

        field<HTMLTextAreaElement>('report-description').value = `https://paste.example/p/${ID}`;
        field<HTMLInputElement>('report-contact').value = 'me@example.org';
        await submit();
        const [post] = posts();
        expect(JSON.parse(post!.body)).toEqual({
            paste_id: ID,
            reason: 'illegal_content',
            description: `https://paste.example/p/${ID}`,
            contact: 'me@example.org',
        });
        for (const request of sent) expect(request.body).not.toContain(KEY);
    });
});
