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
import { type HexagonState, hexagon } from './hexagon';
import {
    byAttr,
    initShell,
    isLeavingPage,
    languageFromNextPage,
    setText,
    showTemplate,
    type Shell,
} from './shell';

// --- phases ------------------------------------------------------------------

/** Shows a phase's template with the state hexagon above its heading. */
function showState(template: string, state: HexagonState): void {
    showTemplate(template, 'data-manage-root');
    document.querySelector('[data-manage-root] .cb-state')!.prepend(hexagon(state));
}

function showLoading(): void {
    showState('tmpl-loading', 'running');
}

function showReady(descriptor: DescriptorResponse, onDelete: () => void): void {
    showState('tmpl-ready', 'warning');

    setText(
        'meta-expires',
        descriptor.expires_at === null
            ? t('manage.neverExpires')
            : t('manage.expiresAt', {
                  date: formatDateTime(new Date(descriptor.expires_at * 1000)),
              }),
    );
    setText('meta-burn', t(descriptor.burn_after_retrieval ? 'manage.burn' : 'manage.noBurn'));
    byAttr('meta-burn').classList.toggle('cb-chip-warning', descriptor.burn_after_retrieval);
    setText('meta-size', t('manage.ciphertextSize', { size: formatSize(descriptor.payload_size) }));

    byAttr('delete-btn').addEventListener('click', onDelete);
}

function showDeleting(): void {
    showState('tmpl-deleting', 'running');
}

function showDone(): void {
    showState('tmpl-done', 'empty');
}

function showError(message: string): void {
    showState('tmpl-error', 'empty');
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
        // The switch is loading this page in another language: the delete would
        // run behind a page that never shows its outcome.
        if (isLeavingPage(shell)) return;
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
