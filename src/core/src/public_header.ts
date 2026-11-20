/**
 * Format and cipher version constants, and the public header shape.
 *
 * This is the reference implementation of the wire format specified in
 * `docs/capsule-file-format.md` and `docs/crypto-profile.md` — those documents
 * are the language-neutral source of truth; this file, and its counterparts in
 * any other client, must match them exactly.
 */

import { fromBase64url, toBase64url } from './helpers/bytes';

/** Always 1. Bumped when the envelope format changes. */
export const FORMAT_VERSION = 1;

/** Always `"capsule-file-v1"`. Names the plaintext container. */
export const CONTENT_FORMAT = 'capsule-file-v1';

/** Framing version inside the capsule binary. */
export const FRAMING_VERSION = 1;

/** Always `"AES-256-GCM"`. */
export const CIPHER = 'AES-256-GCM';

/** Prefix of the URL fragment: `v1.`. */
export const FRAGMENT_PREFIX = 'v1.';

/** Floor for PBKDF2 iterations in a parsed header: an issuer must not be able to
 *  choose a KDF cheap enough to make guessing the password offline trivial. */
export const MIN_PBKDF2_ITERATIONS = 100_000;

/** Hard ceiling for PBKDF2 iterations to prevent client-side DoS. */
export const MAX_PBKDF2_ITERATIONS = 5_000_000;

/** Length of the password salt in bytes. */
export const PASSWORD_SALT_BYTES = 16;

export interface PasswordKdf {
    required: boolean;
    kdf: string;
    /** 16 bytes, base64url-encoded. */
    salt: string;
    iterations: number;
}

export class PublicHeader {
    public readonly envelope_format: number = FORMAT_VERSION;
    public readonly content_format: string = CONTENT_FORMAT;
    public readonly cipher: string = CIPHER;
    /** 12 bytes, base64url-encoded. */
    public readonly nonce: string;
    /** 16 bytes, base64url-encoded. */
    public readonly hkdf_salt: string;
    /** Present only when the paste is password-protected. */
    public readonly password: PasswordKdf | undefined;

    /** Builds a header from freshly generated random nonce and salt. Mirrors
     *  `PublicHeader::new()` in Rust. */
    constructor(nonce: Uint8Array, hkdfSalt: Uint8Array, password?: PasswordKdf) {
        if (nonce.length !== 12) throw new Error('nonce must be 12 bytes');
        if (hkdfSalt.length !== 16) throw new Error('hkdf_salt must be 16 bytes');

        this.nonce = toBase64url(nonce);
        this.hkdf_salt = toBase64url(hkdfSalt);
        this.password = password;
    }

    /**
     * Parses a public header from its JSON representation.
     *
     * Validates that `envelope_format`, `content_format` and `cipher` match the
     * version this client supports, and that `nonce`/`hkdf_salt` decode to the
     * expected byte lengths. Password parameters must be a canonical base64url
     * salt of 16 bytes and an integer iteration count within
     * [{@link MIN_PBKDF2_ITERATIONS}, {@link MAX_PBKDF2_ITERATIONS}]: the header
     * is untrusted input, so the floor applies here rather than in
     * `derivePasswordKey`. Throws on any mismatch.
     *
     * Mirrors `PublicHeader::from_json()` in Rust.
     */
    public static parseFromJson(json: string): PublicHeader {
        const obj: unknown = JSON.parse(json);
        if (typeof obj !== 'object' || obj === null) {
            throw new Error('public header is not a JSON object');
        }

        const h = obj as Record<string, unknown>;

        if (h['envelope_format'] !== FORMAT_VERSION) {
            throw new Error(
                `unsupported envelope_format ${h['envelope_format']}, expected ${FORMAT_VERSION}`,
            );
        }
        if (h['content_format'] !== CONTENT_FORMAT) {
            throw new Error(
                `unsupported content_format ${JSON.stringify(h['content_format'])}, expected ${CONTENT_FORMAT}`,
            );
        }
        if (h['cipher'] !== CIPHER) {
            throw new Error(
                `unsupported cipher ${JSON.stringify(h['cipher'])}, expected ${CIPHER}`,
            );
        }
        if (typeof h['nonce'] !== 'string' || typeof h['hkdf_salt'] !== 'string') {
            throw new Error('public header nonce or hkdf_salt missing');
        }

        const nonceBytes = fromBase64url(h['nonce']);
        const hkdfSaltBytes = fromBase64url(h['hkdf_salt']);

        // Optional password KDF parameters.
        let password: PasswordKdf | undefined;
        const pwd = h['password'];
        if (pwd !== undefined && pwd !== null) {
            if (typeof pwd !== 'object')
                throw new Error('public header password must be an object');
            const pw = pwd as Record<string, unknown>;
            if (pw['required'] !== true) throw new Error('password.required must be true');
            if (pw['kdf'] !== 'PBKDF2-HMAC-SHA-256') throw new Error('unsupported password kdf');
            const salt = pw['salt'];
            if (typeof salt !== 'string' || !isCanonicalBase64url(salt, PASSWORD_SALT_BYTES))
                throw new Error(`password salt must be ${PASSWORD_SALT_BYTES} bytes of base64url`);
            const iterations = pw['iterations'];
            if (
                typeof iterations !== 'number' ||
                !Number.isSafeInteger(iterations) ||
                iterations < MIN_PBKDF2_ITERATIONS ||
                iterations > MAX_PBKDF2_ITERATIONS
            )
                throw new Error(
                    `password iterations must be an integer from ${MIN_PBKDF2_ITERATIONS} to ${MAX_PBKDF2_ITERATIONS}`,
                );
            password = {
                required: true,
                kdf: pw['kdf'],
                salt,
                iterations,
            };
        }

        // Constructing rather than assigning fields validates nonce/hkdf_salt
        // length the same way a freshly built header does.
        return new PublicHeader(nonceBytes, hkdfSaltBytes, password);
    }

    /**
     * Serialises the header to canonical JSON (no whitespace), then to UTF-8
     * bytes — the byte sequence used as AES-GCM AAD.
     *
     * Built explicitly field by field, in the fixed order the spec requires,
     * rather than `JSON.stringify(this)` — the instance must stay free to grow
     * fields for its own convenience without that silently changing the AAD.
     *
     * Mirrors `PublicHeader::to_bytes()` in Rust.
     */
    public toAad(): Uint8Array {
        const wire: Record<string, unknown> = {
            envelope_format: this.envelope_format,
            content_format: this.content_format,
            cipher: this.cipher,
            nonce: this.nonce,
            hkdf_salt: this.hkdf_salt,
        };
        if (this.password) {
            wire['password'] = this.password;
        }
        return new TextEncoder().encode(JSON.stringify(wire));
    }

    /** Decodes the nonce field to raw bytes. */
    public nonceBytes(): Uint8Array {
        return fromBase64url(this.nonce);
    }

    /** Decodes the hkdf_salt field to raw bytes. */
    public hkdfSaltBytes(): Uint8Array {
        return fromBase64url(this.hkdf_salt);
    }

    /** Returns true when the capsule requires a password. */
    public hasPassword(): boolean {
        return this.password?.required === true;
    }
}

/** Whether `text` is the unpadded base64url encoding of exactly `length` bytes,
 *  as {@link toBase64url} writes it. */
function isCanonicalBase64url(text: string, length: number): boolean {
    if (text.length !== Math.ceil((length * 4) / 3) || !/^[A-Za-z0-9_-]*$/.test(text)) {
        return false;
    }
    const bytes = fromBase64url(text);
    return bytes.length === length && toBase64url(bytes) === text;
}
