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
    isPasteId,
    parseFragment,
    toBase64url,
} from '@capsulebin/core';
import { formatDateTime, formatSize, t } from '../language';
import { apiErrorText, linkErrorText } from './errors';
import {
    byAttr,
    initShell,
    languageFromNextPage,
    setText,
    showTemplate,
    type Shell,
} from './shell';

// --- phases ------------------------------------------------------------------

function showLoading(): void {
    showTemplate('tmpl-loading', 'data-manage-root');
}

function showReady(descriptor: DescriptorResponse, onDelete: () => void): void {
    showTemplate('tmpl-ready', 'data-manage-root');

    setText(
        'meta-expires',
        descriptor.expires_at === null
            ? t('manage.neverExpires')
            : t('manage.expiresAt', {
                  date: formatDateTime(new Date(descriptor.expires_at * 1000)),
              }),
    );
    setText('meta-burn', t(descriptor.burn_after_retrieval ? 'manage.burn' : 'manage.noBurn'));
    setText('meta-size', t('manage.ciphertextSize', { size: formatSize(descriptor.payload_size) }));

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

async function startDelete(shell: Shell): Promise<void> {
    const api = new PasteApi('');

    // 1. The delete secret from the fragment, the id from the path (/manage/<id>).
    let secretB64: string;
    try {
        secretB64 = toBase64url(parseFragment(window.location.hash));
    } catch {
        return showError(linkErrorText(window.location.hash));
    }

    const id = window.location.pathname.split('/').at(-1) ?? '';
    if (!isPasteId(id)) {
        return showError(t('link.invalidId'));
    }

    // 3. Fetch lifecycle metadata so the user knows what they are about to delete.
    let descriptor;
    try {
        descriptor = await api.getDescriptor(id, { deleteSecret: secretB64 });
    } catch (err) {
        if (err instanceof ApiError && (err.status === 404 || err.status === 410)) {
            return showError('');
        }
        return showError(apiErrorText(err));
    }

    // 4. Show confirmation.
    showReady(descriptor, async () => {
        // From here on reloading the page in another language would lose the key,
        // or leave the delete running behind a page that never learns its outcome.
        languageFromNextPage(shell);
        showDeleting();

        try {
            await api.delete(id, secretB64);
        } catch (err) {
            if (err instanceof ApiError && (err.status === 404 || err.status === 403)) {
                return showError('');
            }
            return showError(apiErrorText(err));
        }

        // Clear the fragment.
        history.replaceState(null, '', window.location.pathname);
        showDone();
    });
}

// --- boot --------------------------------------------------------------------

const shell = initShell();
showLoading();
void startDelete(shell);
