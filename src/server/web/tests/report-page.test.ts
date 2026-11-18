/**
 * The report page wired to its real HTML: what is sent, and that the key in a
 * pasted link never is.
 */

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const KEY = 'v1.c2VjcmV0LWtleS1ub3QtdG8tYmUtc2VudA';

const html = readFileSync(resolve(__dirname, '../report.html'), 'utf8');

interface Sent {
    url: string;
    method: string;
    body: string;
}

let sent: Sent[];

function stubServer(policy: { enabled: boolean; contact: string | null }, reportStatus = 202) {
    sent = [];
    vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
        sent.push({ url, method: init?.method ?? 'GET', body: String(init?.body ?? '') });
        if ((init?.method ?? 'GET') === 'GET') {
            return new Response(JSON.stringify(policy), { status: 200 });
        }
        return new Response(JSON.stringify({ status: 'received', code: 'x' }), {
            status: reportStatus,
        });
    });
}

async function loadPage(search = ''): Promise<void> {
    document.documentElement.innerHTML = new DOMParser().parseFromString(
        html,
        'text/html',
    ).documentElement.innerHTML;
    window.history.replaceState(null, '', `/report${search}`);
    vi.resetModules();
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
