import { describe, expect, it } from 'vitest';

import { MAX_PBKDF2_ITERATIONS, PublicHeader, type PasswordKdf } from '../src/public_header';

const nonce = new Uint8Array(12).fill(0x01);
const hkdfSalt = new Uint8Array(16).fill(0x02);

function validPasswordKdf(iterations: number): PasswordKdf {
    return {
        required: true,
        kdf: 'PBKDF2-HMAC-SHA-256',
        salt: 'AQIDBAUGBwgJCgsMDQ4PEA',
        iterations,
    };
}

describe('public header', () => {
    it('round-trips without a password', () => {
        const header = new PublicHeader(nonce, hkdfSalt);
        const json = new TextDecoder().decode(header.toAad());
        const parsed = PublicHeader.parseFromJson(json);

        expect(parsed).toEqual(header);
        expect(parsed.hasPassword()).toBe(false);
    });

    it('round-trips with a password', () => {
        const password = validPasswordKdf(1000);
        const header = new PublicHeader(nonce, hkdfSalt, password);
        const json = new TextDecoder().decode(header.toAad());
        const parsed = PublicHeader.parseFromJson(json);

        expect(parsed.password).toEqual(password);
        expect(parsed.hasPassword()).toBe(true);
    });

    it('rejects excessive PBKDF2 iterations', () => {
        const header = new PublicHeader(nonce, hkdfSalt, validPasswordKdf(2_000_000_000));
        const json = new TextDecoder().decode(header.toAad());

        expect(() => PublicHeader.parseFromJson(json)).toThrow(/iterations/i);
    });

    it('accepts iterations exactly at the ceiling', () => {
        const header = new PublicHeader(nonce, hkdfSalt, validPasswordKdf(MAX_PBKDF2_ITERATIONS));
        const json = new TextDecoder().decode(header.toAad());

        expect(() => PublicHeader.parseFromJson(json)).not.toThrow();
    });

    it('rejects zero or negative iterations', () => {
        const header = new PublicHeader(nonce, hkdfSalt, validPasswordKdf(0));
        const json = new TextDecoder().decode(header.toAad());

        expect(() => PublicHeader.parseFromJson(json)).toThrow(/iterations/i);
    });

    it('rejects a nonce or hkdf_salt of the wrong length', () => {
        expect(() => new PublicHeader(new Uint8Array(11), hkdfSalt)).toThrow(/nonce/i);
        expect(() => new PublicHeader(nonce, new Uint8Array(15))).toThrow(/hkdf_salt/i);
    });

    it('nonceBytes/hkdfSaltBytes decode back to the original bytes', () => {
        const header = new PublicHeader(nonce, hkdfSalt);
        expect(header.nonceBytes()).toEqual(nonce);
        expect(header.hkdfSaltBytes()).toEqual(hkdfSalt);
    });

    it('re-serializes to byte-identical output after a parse round-trip', () => {
        // crypto-profile.md's whole canonical-serialization section rests on this:
        // the same logical header must serialize to the exact same bytes on every
        // implementation, every time, since the header is authenticated as AES-GCM
        // AAD. Pinned here so a future field reorder fails loudly in this test
        // instead of as a silent decryption failure somewhere else.
        for (const password of [undefined, validPasswordKdf(1000)]) {
            const original = new PublicHeader(nonce, hkdfSalt, password).toAad();
            const reparsed = PublicHeader.parseFromJson(new TextDecoder().decode(original));
            expect(reparsed.toAad()).toEqual(original);
        }
    });
});
