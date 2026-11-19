/**
 * Report page: notice and action for one capsule (ADR-013).
 *
 * The reporter may paste a full share link. Only the id leaves the browser —
 * the fragment (the decryption key) is cut off before anything else looks at
 * the text, and the server refuses descriptions that still contain one.
 *
 * The server answers every well-formed report the same way, whether or not the
 * capsule exists, so this page always shows the same confirmation.
 */

import { pasteIdFromInput } from '@capsulebin/core';
import { t, type TextKey } from '../language';
import { byAttr, initShell, setText, showTemplate } from './shell';

const ROOT = 'data-report-root';

/**
 * The text for each problem `code` the report endpoint answers with (including
 * the rate limiter and the body limit in front of it). The server's `detail` is
 * English and never shown; an unknown code gets the general text.
 */
const PROBLEM_TEXTS: Readonly<Record<string, TextKey>> = {
    malformed_request: 'report.error.rejected',
    invalid_id: 'report.error.invalidId',
    payload_too_large: 'report.error.tooLarge',
    rate_limited: 'report.error.rateLimited',
    internal_error: 'report.error.server',
};

/** The message for a refused report; `null` for `reports_disabled`, which has its own page state. */
function problemText(status: number, code: string | undefined): TextKey | null {
    if (code === 'reports_disabled') return null;
    if (status === 429) return 'report.error.rateLimited';
    return (code && PROBLEM_TEXTS[code]) || 'report.error.generic';
}

interface ReportPolicy {
    enabled: boolean;
    contact: string | null;
}

async function loadPolicy(): Promise<ReportPolicy> {
    try {
        const response = await fetch('/api/v1/reports');
        if (!response.ok) throw new Error(String(response.status));
        const body = (await response.json()) as Partial<ReportPolicy>;
        return { enabled: body.enabled !== false, contact: body.contact ?? null };
    } catch {
        // Offer the form anyway; the submission will report what went wrong.
        return { enabled: true, contact: null };
    }
}

function showError(message: string): void {
    const el = byAttr('report-error');
    el.textContent = message;
    el.hidden = false;
}

async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const button = byAttr<HTMLButtonElement>('report-submit');
    byAttr('report-error').hidden = true;

    const pasteField = byAttr<HTMLInputElement>('report-paste');
    const pasteId = pasteIdFromInput(pasteField.value);
    if (!pasteId) {
        return showError(t('report.error.noId'));
    }
    // Never leave the key in the field, even if the request fails.
    pasteField.value = pasteId;

    const description = byAttr<HTMLTextAreaElement>('report-description').value.trim();
    if (!description) {
        return showError(t('report.error.noDescription'));
    }
    const contact = byAttr<HTMLInputElement>('report-contact').value.trim();

    button.disabled = true;
    try {
        const response = await fetch('/api/v1/reports', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                paste_id: pasteId,
                reason: byAttr<HTMLSelectElement>('report-reason').value,
                description,
                ...(contact ? { contact } : {}),
            }),
        });
        if (response.status === 202) {
            showTemplate('tmpl-sent', ROOT);
            return;
        }
        const problem = (await response.json().catch(() => ({}))) as { code?: unknown };
        const text = problemText(
            response.status,
            typeof problem.code === 'string' ? problem.code : undefined,
        );
        if (text) showError(t(text));
        else showTemplate('tmpl-disabled', ROOT);
    } catch {
        showError(t('report.error.network'));
    } finally {
        button.disabled = false;
    }
}

async function start(): Promise<void> {
    const policy = await loadPolicy();
    if (!policy.enabled) {
        showTemplate('tmpl-disabled', ROOT);
        return;
    }

    showTemplate('tmpl-form', ROOT);

    // Prefill from `?id=` (the view page's report link). Query, not fragment:
    // a fragment would be where a key lives.
    const fromQuery = pasteIdFromInput(new URLSearchParams(window.location.search).get('id') ?? '');
    if (fromQuery) byAttr<HTMLInputElement>('report-paste').value = fromQuery;

    if (policy.contact) {
        setText('report-contact-address', policy.contact);
        byAttr('report-contact-note').hidden = false;
    }

    byAttr<HTMLFormElement>('report-form').addEventListener('submit', (event) => {
        void submit(event as SubmitEvent);
    });
}

initShell();
void start();
