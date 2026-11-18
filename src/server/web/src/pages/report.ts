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
import { byAttr, initShell, setText, showTemplate } from './shell';

const ROOT = 'data-report-root';

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
        return showError('Enter a share link or a 22-character paste id.');
    }
    // Never leave the key in the field, even if the request fails.
    pasteField.value = pasteId;

    const description = byAttr<HTMLTextAreaElement>('report-description').value.trim();
    if (!description) {
        return showError('Please describe the problem.');
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
        const problem = (await response.json().catch(() => ({}))) as {
            code?: string;
            detail?: string;
        };
        if (response.status === 429) {
            showError('Too many reports from your network. Please try again later.');
        } else if (problem.code === 'reports_disabled') {
            showTemplate('tmpl-disabled', ROOT);
        } else if (problem.detail) {
            showError(`The report was not accepted: ${problem.detail}.`);
        } else {
            showError('The report could not be sent. Please try again later.');
        }
    } catch {
        showError('The report could not be sent. Check your connection and try again.');
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
