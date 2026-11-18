/**
 * Tests for the generic crypto primitives.
 *
 * These tests use the actual Web Crypto API. They run under jsdom, which in
 * recent Node.js versions exposes crypto.subtle from the runtime.
 */

import { describe, expect, it } from 'vitest';
import {
    decrypt,
    deriveBits,
    derivePasswordKey,
    encrypt,
    importAesGcmKey,
    randomBytes,
    sha256,
} from '../src/crypto';
import { fromBase64url, toBase64url } from '../src/helpers/bytes';

/** Converts a Uint8Array to a plain array for deep-equal comparison. */
function toArray(bytes: Uint8Array): number[] {
    return Array.from(bytes);
}

describe('randomBytes', () => {
    it('generates the requested length', () => {
        expect(randomBytes(32)).toHaveLength(32);
        expect(randomBytes(12)).toHaveLength(12);
    });

    it('generates distinct output', () => {
        const a = randomBytes(32);
        const b = randomBytes(32);
        // Cryptographic randomness: two calls must not produce identical output.
        expect(a).not.toEqual(b);
    });
});

describe('base64url', () => {
    it('round-trips bytes through base64url', () => {
        const original = new Uint8Array([0x00, 0xff, 0x7f, 0x80, 0x41]);
        const encoded = toBase64url(original);
        // Must not contain standard base64 characters.
        expect(encoded).not.toContain('+');
        expect(encoded).not.toContain('/');
        expect(encoded).not.toContain('=');
        const decoded = fromBase64url(encoded);
        expect(decoded).toEqual(original);
    });

    it('decodes base64url with and without padding', () => {
        // "AA" in base64url is 1 byte (0x00) with 4 padding bits.
        const fromPadded = fromBase64url('AA==');
        const fromUnpadded = fromBase64url('AA');
        expect(fromPadded).toEqual(new Uint8Array([0x00]));
        expect(fromUnpadded).toEqual(new Uint8Array([0x00]));
    });
});

describe('sha256', () => {
    it('hashes known input to known output', async () => {
        // SHA-256("abc") is well-known.
        const hash = await sha256(new TextEncoder().encode('abc'));
        const hex = Array.from(hash)
            .map((b) => b.toString(16).padStart(2, '0'))
            .join('');
        expect(hex).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    });
});

describe('deriveBits', () => {
    it('produces the requested length', async () => {
        const secret = randomBytes(32);
        const salt = randomBytes(16);
        const info = new TextEncoder().encode('test/info');

        const bits = await deriveBits(secret, salt, info);
        expect(bits).toHaveLength(32);
    });

    it('different info labels produce unrelated output from the same secret', async () => {
        const secret = randomBytes(32);
        const salt = randomBytes(16);

        const a = await deriveBits(secret, salt, new TextEncoder().encode('a'));
        const b = await deriveBits(secret, salt, new TextEncoder().encode('b'));
        expect(a).not.toEqual(b);
    });

    it('is deterministic for the same inputs', async () => {
        const secret = new Uint8Array(32).fill(0x42);
        const salt = new Uint8Array(16).fill(0x13);
        const info = new TextEncoder().encode('info');

        const a = await deriveBits(secret, salt, info);
        const b = await deriveBits(secret, salt, info);
        expect(a).toEqual(b);
    });

    it('produces different output for different salts', async () => {
        const secret = new Uint8Array(32).fill(0x42);
        const info = new TextEncoder().encode('info');
        const saltA = new Uint8Array(16).fill(0x13);
        const saltB = new Uint8Array(16).fill(0x14);

        const a = await deriveBits(secret, saltA, info);
        const b = await deriveBits(secret, saltB, info);
        expect(a).not.toEqual(b);
    });

    it('mixes a password key into the derivation when given', async () => {
        const secret = new Uint8Array(32).fill(0x42);
        const salt = new Uint8Array(16).fill(0x13);
        const info = new TextEncoder().encode('info');
        const passwordKey = new Uint8Array(32).fill(0x99);

        const withoutPassword = await deriveBits(secret, salt, info);
        const withPassword = await deriveBits(secret, salt, info, passwordKey);
        expect(withPassword).not.toEqual(withoutPassword);
    });
});

describe('importAesGcmKey', () => {
    it('imports derived bits into a key that can encrypt and decrypt', async () => {
        const secret = randomBytes(32);
        const salt = randomBytes(16);
        const info = new TextEncoder().encode('test/info');
        const nonce = randomBytes(12);
        const aad = new TextEncoder().encode('test aad');
        const plaintext = new TextEncoder().encode('hello capsule');

        const bits = await deriveBits(secret, salt, info);
        const key = await importAesGcmKey(bits);
        const ciphertext = await encrypt(key, nonce, plaintext, aad);

        // Encrypt is not deterministic (GCM includes a counter), but decrypt must
        // recover the plaintext exactly.
        const decrypted = await decrypt(key, nonce, ciphertext, aad);
        expect(toArray(decrypted)).toEqual(toArray(plaintext));
    });
});

describe('encrypt / decrypt', () => {
    it('fails decryption with wrong AAD', async () => {
        const key = await importAesGcmKey(randomBytes(32));
        const nonce = randomBytes(12);
        const plaintext = new TextEncoder().encode('secret');

        const ciphertext = await encrypt(
            key,
            nonce,
            plaintext,
            new TextEncoder().encode('correct aad'),
        );

        // Decrypt with wrong AAD must throw.
        await expect(
            decrypt(key, nonce, ciphertext, new TextEncoder().encode('wrong aad')),
        ).rejects.toThrow();
    });

    it('fails decryption with wrong key', async () => {
        const keyA = await importAesGcmKey(randomBytes(32));
        const keyB = await importAesGcmKey(randomBytes(32));
        const nonce = randomBytes(12);
        const aad = new TextEncoder().encode('aad');
        const plaintext = new TextEncoder().encode('secret');

        const ciphertext = await encrypt(keyA, nonce, plaintext, aad);

        await expect(decrypt(keyB, nonce, ciphertext, aad)).rejects.toThrow();
    });
});

describe('PBKDF2 password derivation', () => {
    it('derives a 32-byte key from password + salt', async () => {
        const salt = new Uint8Array(16).fill(0x01);
        const key = await derivePasswordKey('test-password', salt, 1000);
        expect(key).toHaveLength(32);
    });

    it('produces different keys for different passwords', async () => {
        const salt = new Uint8Array(16).fill(0x02);
        const a = await derivePasswordKey('password-a', salt, 1000);
        const b = await derivePasswordKey('password-b', salt, 1000);
        expect(a).not.toEqual(b);
    });

    it('produces different keys for different salts', async () => {
        const pw = 'same-password';
        const saltA = new Uint8Array(16).fill(0x03);
        const saltB = new Uint8Array(16).fill(0x04);
        const a = await derivePasswordKey(pw, saltA, 1000);
        const b = await derivePasswordKey(pw, saltB, 1000);
        expect(a).not.toEqual(b);
    });

    it('is deterministic', async () => {
        const salt = new Uint8Array(16).fill(0x05);
        const a = await derivePasswordKey('deterministic', salt, 1000);
        const b = await derivePasswordKey('deterministic', salt, 1000);
        expect(a).toEqual(b);
    });
});
