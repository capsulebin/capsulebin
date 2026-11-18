/**
 * Recipient-side decrypt flow for the web view: reads the link from
 * `window.location` and hands the sequence of network calls and crypto
 * derivations to `openPaste()` in `@capsulebin/core` — the same flow
 * `capsulebin get` uses. No Lit/DOM state here: `view.ts` owns the phase
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
    LinkError,
    openPaste,
    PasteApi,
    parseFragment,
    isPasteId,
    type PublicHeader,
} from '@capsulebin/core';

export interface OpenCapsuleHooks {
    /** About to derive keys and decrypt; lets the caller show a spinner. */
    onDecrypting(): void;
    /** Show the burn-once warning; resolves once the user confirms. */
    confirmBurn(): Promise<void>;
    /** Show the password prompt; resolves with the derived password key. */
    promptForPassword(header: PublicHeader): Promise<Uint8Array>;
}

export type OpenCapsuleOutcome =
    | {
          kind: 'success';
          title: string | null;
          items: readonly CapsuleItem[];
          expiresAt: number | null;
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
    } catch (err) {
        return { kind: 'error', message: linkMessage(err), final: false };
    }

    const id = window.location.pathname.split('/').at(-1) ?? '';
    if (!isPasteId(id)) {
        return { kind: 'error', message: 'Invalid capsule identifier.', final: false };
    }

    // 2. Descriptor → (burn, password) → reveal → decrypt → decode, in core.
    let askedForPassword = false;
    const outcome = await openPaste(new PasteApi(''), id, linkSecret, {
        confirmBurn: () => hooks.confirmBurn(),
        passwordKey: (header) => {
            askedForPassword = true;
            return hooks.promptForPassword(header);
        },
        onDecrypting: () => hooks.onDecrypting(),
    });

    switch (outcome.kind) {
        case 'success':
            return {
                kind: 'success',
                title: outcome.capsule.title,
                items: outcome.capsule.items,
                expiresAt: outcome.expiresAt,
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
                message:
                    'Decryption failed. The content may have been modified or the key is wrong.',
                final: true,
            };
        case 'invalid_capsule':
            return { kind: 'error', message: outcome.message, final: true };
    }
}

function linkMessage(err: unknown): string {
    if (!(err instanceof LinkError)) return 'The link could not be read.';
    const text = err.message;
    return text.charAt(0).toUpperCase() + text.slice(1) + '.';
}
