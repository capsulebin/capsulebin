/**
 * Management page: delete a capsule via its delete secret.
 *
 * The delete secret arrives in the URL fragment (`#v1.<base64url>`) and is
 * never sent as part of the path. The server compares its hash against the
 * stored delete verifier.
 */

import {
    ApiError,
    PasteApi,
    type DescriptorResponse,
    formatSize,
    isPasteId,
    LinkError,
    parseFragment,
    toBase64url,
} from '@capsulebin/core';
import { byAttr, initShell, setText, showTemplate } from './shell';

// --- phases ------------------------------------------------------------------

function showLoading(): void {
    showTemplate('tmpl-loading', 'data-manage-root');
}

function showReady(descriptor: DescriptorResponse, onDelete: () => void): void {
    showTemplate('tmpl-ready', 'data-manage-root');

    setText(
        'meta-expires',
        descriptor.expires_at === null
            ? 'Never expires'
            : `Expires ${new Date(descriptor.expires_at * 1000).toLocaleString()}`,
    );
    setText(
        'meta-burn',
        descriptor.burn_after_retrieval
            ? 'Deletes itself after first retrieval'
            : 'Can be retrieved multiple times',
    );
    setText('meta-size', `${formatSize(descriptor.payload_size)} of ciphertext`);

    byAttr('delete-btn').addEventListener('click', onDelete);
}

function showDeleting(): void {
    showTemplate('tmpl-deleting', 'data-manage-root');
}

function showDone(): void {
    showTemplate('tmpl-done', 'data-manage-root');
}

function showError(message: string): void {
    showTemplate('tmpl-error', 'data-manage-root');
    if (message) setText('error-message', message);
}

// --- flow --------------------------------------------------------------------

async function startDelete(): Promise<void> {
    const api = new PasteApi('');

    // 1. The delete secret from the fragment, the id from the path (/manage/<id>).
    let secretB64: string;
    try {
        secretB64 = toBase64url(parseFragment(window.location.hash));
    } catch (err) {
        const text = err instanceof LinkError ? err.message : 'the link could not be read';
        return showError(text.charAt(0).toUpperCase() + text.slice(1) + '.');
    }

    const id = window.location.pathname.split('/').at(-1) ?? '';
    if (!isPasteId(id)) {
        return showError('Invalid capsule identifier.');
    }

    // 3. Fetch lifecycle metadata so the user knows what they are about to delete.
    let descriptor;
    try {
        descriptor = await api.getDescriptor(id, { deleteSecret: secretB64 });
    } catch (err) {
        if (err instanceof ApiError && (err.status === 404 || err.status === 410)) {
            return showError('');
        }
        return showError(err instanceof Error ? err.message : '');
    }

    // 4. Show confirmation.
    showReady(descriptor, async () => {
        showDeleting();

        try {
            await api.delete(id, secretB64);
        } catch (err) {
            if (err instanceof ApiError && (err.status === 404 || err.status === 403)) {
                return showError('');
            }
            return showError(err instanceof Error ? err.message : '');
        }

        // Clear the fragment.
        history.replaceState(null, '', window.location.pathname);
        showDone();
    });
}

// --- boot --------------------------------------------------------------------

initShell();
showLoading();
startDelete();
