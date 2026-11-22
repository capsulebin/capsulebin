/**
 * Recipient-side decrypt flow for the web view: reads the link from
 * `window.location` and hands the sequence of network calls and crypto
 * derivations to `openPaste()` in `@capsulebin/core` — the same flow
 * `capsulebin-cli get` uses. No Lit/DOM state here: `view.ts` owns the phase
 * state machine and templates.
 *
 * The two UI pauses (burn warning, password prompt) are modeled as hooks the
 * caller supplies, so this function can run to completion without knowing
 * how either is rendered.
 *
 * Every failure path returns a generic message: this function must not let
 * a caller distinguish "expired" from "wrong key" from "never existed" (see
 * ADR-001).
 */

import {
    type CapsuleItem,
    openPaste,
    PasteApi,
    parseFragment,
    isPasteId,
    type PublicHeader,
} from '@capsulebin/core';

import { t } from '../language';
import { linkErrorText } from './errors';

/**
 * Why the password prompt is shown again. The server answers a wrong password
 * like a capsule that went away (ADR-001), so `rejected` covers both.
 */
export type PasswordRetry =
    { reason: 'rejected' } | { reason: 'rate_limited'; retryAfter: number | null };

export interface OpenCapsuleHooks {
    /** About to derive keys and decrypt; lets the caller show progress. */
    onDecrypting(): void;
    /**
     * Right before the retrieve request, also when neither pause below came
     * first. `false` stops the flow there: nothing is sent.
     */
    beforeRetrieve(): boolean;
    /** Show the burn-once warning; resolves once the user confirms. */
    confirmBurn(): Promise<void>;
    /**
     * Show the password prompt; resolves with the derived password key. Called
     * again with `retry` when the server refused the previous one.
     */
    promptForPassword(header: PublicHeader, retry?: PasswordRetry): Promise<Uint8Array>;
}

export type OpenCapsuleOutcome =
    | {
          kind: 'success';
          title: string | null;
          items: readonly CapsuleItem[];
          expiresAt: number | null;
          /** From the descriptor: this retrieval deleted the capsule. */
          burnAfterRetrieval: boolean;
      }
    | { kind: 'burned' }
    | {
          kind: 'error';
          message: string;
          /**
           * The link cannot work any more, so its key need not stay in the
           * address bar. False where retrying makes sense: network and server
           * errors, rate limits, and anything after a password was entered (a
           * wrong password looks exactly like an unavailable capsule).
           */
          final: boolean;
      }
    // A hook rejected after already leaving the UI in a terminal state (or the
    // user navigated away); there is nothing left for the caller to do.
    | { kind: 'aborted' };

export async function openCapsule(hooks: OpenCapsuleHooks): Promise<OpenCapsuleOutcome> {
    // 1. The secret from the fragment, the id from the path (/p/<id>).
    let linkSecret: Uint8Array;
    try {
        linkSecret = parseFragment(window.location.hash);
    } catch {
        return { kind: 'error', message: linkErrorText(window.location.hash), final: false };
    }

    const id = window.location.pathname.split('/').at(-1) ?? '';
    if (!isPasteId(id)) {
        return { kind: 'error', message: t('link.invalidId'), final: false };
    }

    // 2. Descriptor → (burn, password) → reveal → decrypt → decode, in core.
    //    A refusal after a password was entered goes back to the prompt with the
    //    reason instead of ending on "not available". If the capsule really went
    //    away, the next round's descriptor request says so and ends the loop.
    let burnConfirmed = false;
    let retry: PasswordRetry | undefined;
    let askedForPassword: boolean;
    let outcome: Awaited<ReturnType<typeof openPaste>>;
    for (;;) {
        askedForPassword = false;
        outcome = await openPaste(new PasteApi(''), id, linkSecret, {
            confirmBurn: async () => {
                if (burnConfirmed) return;
                await hooks.confirmBurn();
                burnConfirmed = true;
            },
            passwordKey: (header) => {
                askedForPassword = true;
                return hooks.promptForPassword(header, retry);
            },
            onDecrypting: () => hooks.onDecrypting(),
            beforeRetrieve: () => hooks.beforeRetrieve(),
        });
        if (outcome.kind !== 'api_error' || !askedForPassword) break;
        const { status, retryAfter } = outcome.error;
        if (status === 404) retry = { reason: 'rejected' };
        else if (status === 429) retry = { reason: 'rate_limited', retryAfter };
        else break;
    }

    switch (outcome.kind) {
        case 'success':
            return {
                kind: 'success',
                title: outcome.capsule.title,
                items: outcome.capsule.items,
                expiresAt: outcome.expiresAt,
                burnAfterRetrieval: outcome.burnAfterRetrieval,
            };
        case 'burned':
        case 'aborted':
            return { kind: outcome.kind };
        case 'api_error': {
            // Generic on purpose: the view shows its default "not available" text.
            const gone = outcome.error.status === 404 || outcome.error.status === 410;
            return { kind: 'error', message: '', final: gone && !askedForPassword };
        }
        case 'password_required':
            return { kind: 'error', message: '', final: false };
        case 'decrypt_failed':
            return {
                kind: 'error',
                message: t('view.decryptFailed'),
                final: true,
            };
        case 'invalid_capsule':
            // Core's message is English and names format details; the page says it plainly.
            return { kind: 'error', message: t('view.invalidCapsule'), final: true };
    }
}
