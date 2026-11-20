import { describe, expect, it } from 'vitest';

import {
    MAX_PBKDF2_ITERATIONS,
    MIN_PBKDF2_ITERATIONS,
    PublicHeader,
    type PasswordKdf,
} from '../src/public_header';

const nonce = new Uint8Array(12).fill(0x01);
const hkdfSalt = new Uint8Array(16).fill(0x02);

const SALT_16 = 'AQIDBAUGBwgJCgsMDQ4PEA';

function validPasswordKdf(iterations: number, salt = SALT_16): PasswordKdf {
    return {
        required: true,
        kdf: 'PBKDF2-HMAC-SHA-256',
        salt,
        iterations,
    };
}

/** The JSON a server would hand out for a header with these password parameters. */
function headerJson(password: PasswordKdf): string {
    return new TextDecoder().decode(new PublicHeader(nonce, hkdfSalt, password).toAad());
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
        const password = validPasswordKdf(MIN_PBKDF2_ITERATIONS);
        const header = new PublicHeader(nonce, hkdfSalt, password);
        const json = new TextDecoder().decode(header.toAad());
        const parsed = PublicHeader.parseFromJson(json);

        expect(parsed.password).toEqual(password);
        expect(parsed.hasPassword()).toBe(true);
    });

    it('has its iteration floor at 100 000', () => {
        expect(MIN_PBKDF2_ITERATIONS).toBe(100_000);
    });

    it('accepts iterations exactly at the floor and at the ceiling', () => {
        for (const iterations of [MIN_PBKDF2_ITERATIONS, MAX_PBKDF2_ITERATIONS]) {
            const parsed = PublicHeader.parseFromJson(headerJson(validPasswordKdf(iterations)));
            expect(parsed.password?.iterations).toBe(iterations);
        }
    });

    it('rejects iterations below the floor, above the ceiling or not an integer', () => {
        for (const iterations of [
            99_999,
            1000,
            1,
            0,
            -1,
            5_000_001,
            2_000_000_000,
            100_000.5,
            Number.MAX_SAFE_INTEGER + 1,
        ]) {
            expect(() =>
                PublicHeader.parseFromJson(headerJson(validPasswordKdf(iterations))),
            ).toThrow(/iterations/i);
        }
        const asString = headerJson(validPasswordKdf(MIN_PBKDF2_ITERATIONS)).replace(
            '"iterations":100000',
            '"iterations":"100000"',
        );
        expect(asString).toContain('"100000"');
        expect(() => PublicHeader.parseFromJson(asString)).toThrow(/iterations/i);
    });

    it('rejects a password salt that is not 16 bytes of base64url', () => {
        for (const salt of [
            'AQIDBAUGBwgJCgsMDQ4P', // 15 bytes
            'AQIDBAUGBwgJCgsMDQ4PEBE', // 17 bytes
            '',
            'AQIDBAUGBwgJCgsMDQ4P+/', // standard base64 alphabet
            'AQIDBAUGBwgJCgsMDQ4PEA==', // padded
            'AQIDBAUGBwgJCgsMDQ4PEB', // non-canonical trailing bits
            'AQIDBAUGBwgJCgsMDQ4P E', // whitespace
        ]) {
            expect(() =>
                PublicHeader.parseFromJson(
                    headerJson(validPasswordKdf(MIN_PBKDF2_ITERATIONS, salt)),
                ),
            ).toThrow(/salt/i);
        }
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
        for (const password of [undefined, validPasswordKdf(MIN_PBKDF2_ITERATIONS)]) {
            const original = new PublicHeader(nonce, hkdfSalt, password).toAad();
            const reparsed = PublicHeader.parseFromJson(new TextDecoder().decode(original));
            expect(reparsed.toAad()).toEqual(original);
        }
    });
});
