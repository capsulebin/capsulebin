/**
 * Recipient-side flow: descriptor → (burn confirmation, password) → reveal
 * token → ciphertext → decrypted, validated capsule.
 *
 * Shared by the web view and `capsulebin-cli get`, so both clients open a capsule
 * through the same sequence of network calls and derivations. The two pauses
 * that need a human (burn confirmation, password entry) are hooks; this module
 * knows nothing about how either is presented.
 *
 * Outcomes are deliberately coarse. Unknown, expired, deleted and
 * wrong-capability capsules all come back as the same `api_error` the server
 * sent (ADR-001), and a wrong password is indistinguishable from a corrupted
 * payload: both end in `decrypt_failed` or, more commonly, in the server
 * refusing the reveal token.
 */

import {
    ApiError,
    MalformedHeaderError,
    MAX_CIPHERTEXT_BYTES,
    type DescriptorResponse,
    type PasteApi,
} from './api';
import { Capsule, CapsuleError, MAX_CAPSULE_BYTES } from './capsule';
import { decrypt, derivePasswordKey } from './crypto';
import { fromBase64url, toBase64url } from './helpers/bytes';
import { derivePasteKeys, deriveDescribeToken } from './paste';
import type { PublicHeader } from './public_header';

export interface OpenHooks {
    /**
     * Called before retrieving a burn-after-retrieval capsule. Resolve to go
     * ahead; reject to stop without consuming it.
     */
    confirmBurn?(descriptor: DescriptorResponse): Promise<void>;
    /**
     * Called when the capsule is password-protected. Resolve with the
     * password key — use {@link derivePasswordKeyFor} — or reject to stop.
     */
    passwordKey?(header: PublicHeader): Promise<Uint8Array>;
    /** About to derive keys and decrypt (e.g. to show progress). */
    onDecrypting?(): void;
    /**
     * Called right before the retrieve request, with nothing asynchronous in
     * between. Return `false` to stop: nothing is sent, the capsule is not
     * consumed.
     */
    beforeRetrieve?(): boolean;
}

export type OpenOutcome =
    | {
          kind: 'success';
          capsule: Capsule;
          expiresAt: number | null;
          burnAfterRetrieval: boolean;
      }
    /** The capsule was burn-after-retrieval and has already been handed out. */
    | { kind: 'burned' }
    /** The server refused (not available, rate limited, network failure …). */
    | { kind: 'api_error'; error: ApiError }
    /** The capsule needs a password and no `passwordKey` hook was given. */
    | { kind: 'password_required' }
    /** Ciphertext did not authenticate: wrong key, wrong password, or tampering. */
    | { kind: 'decrypt_failed' }
    /** Not a valid capsule: its public header failed to parse (malformed, or
     *  password parameters outside the accepted range), or it decrypted to
     *  something that is not a Capsule File Format v1 container. */
    | { kind: 'invalid_capsule'; message: string }
    /** A hook rejected or stopped the retrieval; nothing was retrieved after that point. */
    | { kind: 'aborted' };

/** Derives the password key for `header` (PBKDF2 with the header's salt and
 *  iteration count). Throws if the header carries no password parameters. */
export function derivePasswordKeyFor(header: PublicHeader, password: string): Promise<Uint8Array> {
    const params = header.password;
    if (!params) throw new Error('capsule is not password-protected');
    return derivePasswordKey(password, fromBase64url(params.salt), params.iterations);
}

/** Opens capsule `id` with its 32-byte link secret. */
export async function openPaste(
    api: PasteApi,
    id: string,
    linkSecret: Uint8Array,
    hooks: OpenHooks = {},
): Promise<OpenOutcome> {
    let descriptor: DescriptorResponse;
    let describeToken: Uint8Array<ArrayBuffer>;
    try {
        describeToken = await deriveDescribeToken(linkSecret, id);
        descriptor = await api.getDescriptor(id, { describeToken });
    } catch (err) {
        // Checked before anything uses the header: no password is asked for
        // and no reveal token is derived from parameters the client refuses.
        if (err instanceof MalformedHeaderError) {
            return { kind: 'invalid_capsule', message: 'the public header is invalid' };
        }
        return apiFailure(err);
    }

    if (descriptor.burn_after_retrieval && hooks.confirmBurn) {
        try {
            await hooks.confirmBurn(descriptor);
        } catch {
            return { kind: 'aborted' };
        }
    }

    const header = descriptor.public_header;

    let passwordKey: Uint8Array | undefined;
    if (header.hasPassword()) {
        if (!hooks.passwordKey) return { kind: 'password_required' };
        try {
            passwordKey = await hooks.passwordKey(header);
        } catch {
            return { kind: 'aborted' };
        }
    }

    hooks.onDecrypting?.();

    let keys;
    try {
        keys = await derivePasteKeys(linkSecret, header.hkdfSaltBytes(), passwordKey);
    } catch {
        return { kind: 'decrypt_failed' };
    }

    if (hooks.beforeRetrieve?.() === false) return { kind: 'aborted' };

    let retrieved;
    try {
        retrieved = await api.retrieve(id, {
            describeToken: toBase64url(describeToken),
            revealToken: toBase64url(keys.revealToken),
        });
    } catch (err) {
        return apiFailure(err);
    }

    let ciphertext: Uint8Array;
    try {
        ciphertext = fromBase64url(retrieved.ciphertext);
    } catch {
        return { kind: 'decrypt_failed' };
    }
    // No valid capsule is larger; refused before spending time on decryption.
    if (ciphertext.length > MAX_CIPHERTEXT_BYTES) {
        return {
            kind: 'invalid_capsule',
            message: `the ciphertext exceeds the ${MAX_CAPSULE_BYTES} byte capsule limit`,
        };
    }

    let plaintext: Uint8Array;
    try {
        plaintext = await decrypt(
            keys.contentKey,
            header.nonceBytes(),
            ciphertext,
            descriptor.public_header_aad,
        );
    } catch {
        return { kind: 'decrypt_failed' };
    }

    let capsule: Capsule;
    try {
        capsule = Capsule.decode(plaintext);
    } catch (err) {
        return {
            kind: 'invalid_capsule',
            message: err instanceof CapsuleError ? err.message : 'the capsule format is invalid',
        };
    }

    return {
        kind: 'success',
        capsule,
        expiresAt: descriptor.expires_at,
        burnAfterRetrieval: descriptor.burn_after_retrieval,
    };
}

function apiFailure(err: unknown): OpenOutcome {
    if (err instanceof ApiError) {
        return err.status === 410 ? { kind: 'burned' } : { kind: 'api_error', error: err };
    }
    return { kind: 'api_error', error: new ApiError('request failed', 0, null) };
}
