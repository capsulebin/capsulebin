/**
 * Paste domain logic: turns capsule items and options into everything a
 * create request needs, and the key derivation a recipient needs to open
 * one.
 *
 * This is the one place that knows what a link secret, a delete secret or a
 * verifier *mean* for a capsule — everything below is built from the generic
 * primitives in `./crypto`. Callers (the web client, the CLI) build a
 * {@link Paste}, add items, then seal and publish it; they never assemble a
 * public header or derive a key themselves.
 */

import {
    DEFAULT_PBKDF2_ITERATIONS,
    deriveBits,
    derivePasswordKey,
    encrypt,
    importAesGcmKey,
    randomBytes,
    sha256,
    type CryptoKey,
} from './crypto';
import { concatByteArrays, encodeUtf8, fromBase64url, toBase64url } from './helpers/bytes';
import { Capsule, MAX_ITEMS, CapsuleError, type CapsuleItem } from './capsule';
import { FORMAT_VERSION, FRAGMENT_PREFIX, PublicHeader, type PasswordKdf } from './public_header';
import type { PasteApi } from './api';

// --- sizes and domain labels ---------------------------------------------------

const LINK_SECRET_BYTES = 32;
const DELETE_SECRET_BYTES = 32;
const NONCE_BYTES = 12;
const HKDF_SALT_BYTES = 16;
const PASSWORD_SALT_BYTES = 16;
const PASTE_ID_BYTES = 16;

const HKDF_INFO_CONTENT = encodeUtf8('capsule/v1/content');
const HKDF_INFO_REVEAL = encodeUtf8('capsule/v1/reveal');
const HKDF_INFO_DESCRIBE = encodeUtf8('capsule/v1/describe');
const DELETE_VERIFIER_DOMAIN = encodeUtf8('capsule/v1/delete');

export interface PasteKeys {
    contentKey: CryptoKey;
    revealToken: Uint8Array<ArrayBuffer>;
}

/**
 * Derives the two capabilities a paste needs from its link secret: the
 * content encryption key and the reveal token. Domain-separated via
 * different HKDF info labels, so the server — which learns the reveal token
 * on retrieval — has no way to compute the content key from it.
 */
export async function derivePasteKeys(
    linkSecret: Uint8Array,
    hkdfSalt: Uint8Array,
    passwordKey?: Uint8Array,
): Promise<PasteKeys> {
    const [contentKeyBits, revealToken] = await Promise.all([
        deriveBits(linkSecret, hkdfSalt, HKDF_INFO_CONTENT, passwordKey),
        deriveBits(linkSecret, hkdfSalt, HKDF_INFO_REVEAL, passwordKey),
    ]);
    const contentKey = await importAesGcmKey(contentKeyBits);
    return { contentKey, revealToken };
}

/**
 * Derives the describe token: the capability to read a paste's descriptor
 * (public header and lifecycle). Without it the descriptor endpoint answers
 * like for an unknown id, so knowing a paste id alone reveals nothing.
 *
 * HKDF over the link secret with the paste id's 16 raw bytes as salt - not the
 * `hkdf_salt`, which lives in the public header the descriptor returns. No
 * password key: the descriptor is what tells the client a password is needed.
 */
export function deriveDescribeToken(
    linkSecret: Uint8Array,
    pasteId: string,
): Promise<Uint8Array<ArrayBuffer>> {
    return deriveBits(linkSecret, fromBase64url(pasteId), HKDF_INFO_DESCRIBE);
}

/**
 * Derives the reveal verifier from a reveal token.
 *
 * The server stores SHA-256(reveal-token). The client sends the token itself
 * and the server hashes it for comparison.
 */
export function revealVerifier(revealToken: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
    return sha256(revealToken);
}

/**
 * Derives the delete verifier from a delete secret.
 *
 * Domain-separated via a prefix hash — the delete secret is never run
 * through HKDF, so a label is the only way to keep it distinct from a raw
 * SHA-256 of the same bytes.
 */
export function deleteVerifier(secret: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
    return sha256(concatByteArrays(DELETE_VERIFIER_DOMAIN, secret));
}

export interface PasteOptions {
    title?: string | null;
    /** Empty or whitespace-only is treated as "no password". */
    password?: string | null;
    /** Seconds until expiry; `null` asks for a permanent paste; omitted
     *  (`undefined`) leaves the choice to the server's `default_expiry`. */
    expiresIn?: number | null | undefined;
    burnAfterRetrieval: boolean;
}

export interface SealedPaste {
    /** Never sent to the server. Goes in the share URL's fragment. */
    linkSecret: Uint8Array<ArrayBuffer>;
    /** Never sent to the server. Goes in the delete URL's fragment. */
    deleteSecret: Uint8Array<ArrayBuffer>;
    id: string;
    formatVersion: number;
    publicHeader: string; // base64url
    ciphertext: string; // base64url
    revealVerifier: string; // base64url
    deleteVerifier: string; // base64url
    describeVerifier: string; // base64url
    /** `undefined` is omitted from the request: server default expiry. */
    expiresIn: number | null | undefined;
    burnAfterRetrieval: boolean;
}

/**
 * A paste being assembled: add items, then {@link seal} it (or skip straight
 * to {@link publish}, which seals for you) — encapsulates the whole
 * create-side flow behind one object instead of a free function callers
 * have to feed a finished item list.
 */
export class Paste {
    public readonly title: string | null;
    public readonly password: string | null;
    public readonly expiresIn: number | null | undefined;
    public readonly burnAfterRetrieval: boolean;

    #items: CapsuleItem[] = [];
    #sealed: SealedPaste | undefined;

    constructor(options: PasteOptions) {
        this.title = options.title ?? null;
        this.password = options.password ?? null;
        this.expiresIn = options.expiresIn;
        this.burnAfterRetrieval = options.burnAfterRetrieval;
    }

    /** Items added so far. Use {@link addItem} to add one. */
    public get items(): readonly CapsuleItem[] {
        return this.#items;
    }

    /** The result of {@link seal}, once called. */
    public get sealed(): SealedPaste | undefined {
        return this.#sealed;
    }

    /**
     * Adds an item, rejecting once the per-capsule item limit is reached —
     * immediately, rather than only once {@link seal} builds the capsule.
     */
    public addItem(item: CapsuleItem): void {
        if (this.#sealed) {
            throw new Error('cannot add items after the paste has been sealed');
        }
        if (this.#items.length >= MAX_ITEMS) {
            throw new CapsuleError(`a capsule may contain at most ${MAX_ITEMS} items`);
        }
        this.#items.push(item);
    }

    /**
     * Encrypts the current items and assembles everything a create request
     * needs. One-shot: throws if this paste was already sealed — create a
     * new `Paste` to seal a different set of items instead of mutating one
     * that's already been published.
     */
    public async seal(): Promise<SealedPaste> {
        if (this.#sealed) {
            throw new Error('paste has already been sealed');
        }

        const linkSecret = randomBytes(LINK_SECRET_BYTES);
        const deleteSecret = randomBytes(DELETE_SECRET_BYTES);
        const nonce = randomBytes(NONCE_BYTES);
        const hkdfSalt = randomBytes(HKDF_SALT_BYTES);

        let passwordKey: Uint8Array | undefined;
        let passwordKdf: PasswordKdf | undefined;
        if (this.password && this.password.trim()) {
            const passwordSalt = randomBytes(PASSWORD_SALT_BYTES);
            passwordKey = await derivePasswordKey(this.password, passwordSalt);
            passwordKdf = {
                required: true,
                kdf: 'PBKDF2-HMAC-SHA-256',
                salt: toBase64url(passwordSalt),
                iterations: DEFAULT_PBKDF2_ITERATIONS,
            };
        }

        const header = new PublicHeader(nonce, hkdfSalt, passwordKdf);
        const headerBytes = header.toAad();

        const capsuleBytes = new Capsule(this.#items, this.title).encode();

        const { contentKey, revealToken } = await derivePasteKeys(
            linkSecret,
            hkdfSalt,
            passwordKey,
        );
        const ciphertext = await encrypt(contentKey, nonce, capsuleBytes, headerBytes);

        const id = toBase64url(randomBytes(PASTE_ID_BYTES));
        const describeToken = await deriveDescribeToken(linkSecret, id);

        const [revealV, deleteV, describeV] = await Promise.all([
            revealVerifier(revealToken),
            deleteVerifier(deleteSecret),
            // Same construction as the reveal verifier: the token is an HKDF
            // output with its own label, a plain SHA-256 suffices.
            sha256(describeToken),
        ]);

        this.#sealed = {
            linkSecret,
            deleteSecret,
            id,
            formatVersion: FORMAT_VERSION,
            publicHeader: toBase64url(headerBytes),
            ciphertext: toBase64url(ciphertext),
            revealVerifier: toBase64url(revealV),
            deleteVerifier: toBase64url(deleteV),
            describeVerifier: toBase64url(describeV),
            expiresIn: this.expiresIn,
            burnAfterRetrieval: this.burnAfterRetrieval,
        };
        return this.#sealed;
    }

    /** Builds the recipient share URL. `origin` is the scheme+host the
     *  caller is serving from (e.g. `window.location.origin`, or the CLI's
     *  `--server`) — this class has no ambient notion of one. Throws if
     *  called before {@link seal}. */
    public shareUrl(origin: string): string {
        const sealed = this.#requireSealed();
        return `${origin}/p/${sealed.id}#${FRAGMENT_PREFIX}${toBase64url(sealed.linkSecret)}`;
    }

    /** Builds the management (delete) URL. See {@link shareUrl}. */
    public deleteUrl(origin: string): string {
        const sealed = this.#requireSealed();
        return `${origin}/manage/${sealed.id}#${FRAGMENT_PREFIX}${toBase64url(sealed.deleteSecret)}`;
    }

    /**
     * Seals (if not already sealed) and creates the paste via `api`,
     * returning the share and delete URLs relative to `origin`.
     */
    public async publish(
        api: PasteApi,
        origin: string,
    ): Promise<{ shareUrl: string; deleteUrl: string }> {
        const sealed = this.#sealed ?? (await this.seal());
        await api.create(sealed);
        return { shareUrl: this.shareUrl(origin), deleteUrl: this.deleteUrl(origin) };
    }

    #requireSealed(): SealedPaste {
        if (!this.#sealed) throw new Error('paste has not been sealed yet');
        return this.#sealed;
    }
}
