/**
 * Generic client-side cryptographic primitives: HKDF, PBKDF2, AES-256-GCM and
 * SHA-256, wrapping only the Web Crypto API.
 *
 * Nothing here knows about capsules, pastes, link secrets or verifiers — those
 * are paste-domain concepts and live in `./paste`, built on top of the
 * primitives below. This module stays generic so any future domain object
 * can reuse it without dragging capsule-specific constants along.
 */

import type { webcrypto } from 'node:crypto';
import { concatByteArrays, encodeUtf8 } from './helpers/bytes';
type CryptoKey = webcrypto.CryptoKey;

/** Separator byte between an HKDF info label and mixed-in password key material. */
const INFO_SEPARATOR = new Uint8Array([0]);

export const DEFAULT_PBKDF2_ITERATIONS = 600_000;

/** `length` cryptographically secure random bytes. */
export function randomBytes(length: number): Uint8Array<ArrayBuffer> {
    return crypto.getRandomValues(new Uint8Array(length));
}

function hkdfInfo(info: Uint8Array, passwordKey?: Uint8Array): Uint8Array<ArrayBuffer> {
    return passwordKey
        ? (concatByteArrays(info, INFO_SEPARATOR, passwordKey) as Uint8Array<ArrayBuffer>)
        : (info as Uint8Array<ArrayBuffer>);
}

/**
 * Derives raw bytes via HKDF-SHA-256.
 *
 * `info` is a domain-separation label chosen by the caller: the same
 * `secret`/`salt` pair with a different `info` produces unrelated output.
 * When `passwordKey` is given it is mixed into `info` (separated by a single
 * `0x00` byte) so a wrong password key derives different output. Use
 * {@link importAesGcmKey} on the result when the output is an encryption
 * key rather than a token.
 */
export async function deriveBits(
    secret: Uint8Array,
    salt: Uint8Array,
    info: Uint8Array,
    passwordKey?: Uint8Array,
    length = 256,
): Promise<Uint8Array<ArrayBuffer>> {
    const ikm = await crypto.subtle.importKey(
        'raw',
        secret as Uint8Array<ArrayBuffer>,
        { name: 'HKDF' },
        false,
        ['deriveBits'],
    );

    return new Uint8Array(
        await crypto.subtle.deriveBits(
            {
                name: 'HKDF',
                hash: 'SHA-256',
                salt: salt as Uint8Array<ArrayBuffer>,
                info: hkdfInfo(info, passwordKey),
            },
            ikm,
            length,
        ),
    );
}

/**
 * Derives a 32-byte key via PBKDF2-HMAC-SHA-256 from a user-supplied password.
 *
 * The password is normalized to Unicode NFC first: the same password typed on
 * different systems can arrive as different code point sequences (an "ä" as
 * one code point, or as "a" plus a combining diaeresis), and without
 * normalization those would derive different keys and the capsule would not
 * open (crypto profile v1).
 */
export async function derivePasswordKey(
    password: string,
    salt: Uint8Array,
    iterations: number = DEFAULT_PBKDF2_ITERATIONS,
): Promise<Uint8Array<ArrayBuffer>> {
    const ikm = await crypto.subtle.importKey(
        'raw',
        encodeUtf8(password.normalize('NFC')),
        { name: 'PBKDF2' },
        false,
        ['deriveBits'],
    );
    return new Uint8Array(
        await crypto.subtle.deriveBits(
            {
                name: 'PBKDF2',
                hash: 'SHA-256',
                salt: salt as Uint8Array<ArrayBuffer>,
                iterations,
            },
            ikm,
            256,
        ),
    );
}

/** Imports raw bytes (e.g. from {@link deriveBits}) as an AES-256-GCM key. */
export function importAesGcmKey(bytes: Uint8Array): Promise<CryptoKey> {
    return crypto.subtle.importKey(
        'raw',
        bytes as Uint8Array<ArrayBuffer>,
        { name: 'AES-GCM' },
        true,
        ['encrypt', 'decrypt'],
    );
}

/**
 * Encrypts plaintext with AES-256-GCM.
 *
 * `aad` (additional authenticated data) is authenticated together with the
 * ciphertext, so any modification to it is detected during decryption.
 */
export async function encrypt(
    key: CryptoKey,
    nonce: Uint8Array,
    plaintext: Uint8Array,
    aad: Uint8Array,
): Promise<Uint8Array<ArrayBuffer>> {
    return new Uint8Array(
        await crypto.subtle.encrypt(
            {
                name: 'AES-GCM',
                iv: nonce as Uint8Array<ArrayBuffer>,
                additionalData: aad as Uint8Array<ArrayBuffer>,
            },
            key,
            plaintext as Uint8Array<ArrayBuffer>,
        ),
    );
}

/**
 * Decrypts ciphertext with AES-256-GCM.
 *
 * Throws if authentication fails — the ciphertext or AAD was modified, or the
 * wrong key was used. All three produce the same generic error so that an
 * observer cannot distinguish them.
 */
export async function decrypt(
    key: CryptoKey,
    nonce: Uint8Array,
    ciphertext: Uint8Array,
    aad: Uint8Array,
): Promise<Uint8Array<ArrayBuffer>> {
    return new Uint8Array(
        await crypto.subtle.decrypt(
            {
                name: 'AES-GCM',
                iv: nonce as Uint8Array<ArrayBuffer>,
                additionalData: aad as Uint8Array<ArrayBuffer>,
            },
            key,
            ciphertext as Uint8Array<ArrayBuffer>,
        ),
    );
}

/** SHA-256 of arbitrary bytes. */
export async function sha256(data: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
    return new Uint8Array(await crypto.subtle.digest('SHA-256', data as Uint8Array<ArrayBuffer>));
}
