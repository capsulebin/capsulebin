/**
 * Property-based tests for every parser that reads untrusted input: the
 * capsule decoder (decrypted bytes may come from a malicious creator), the
 * public header (comes from the server), base64url, and links (pasted by
 * users).
 *
 * The properties are the ones a fuzzer would check: round trips hold, and
 * arbitrary input fails only in the documented way — never with a TypeError,
 * RangeError or an attempt to allocate based on an unchecked size.
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';
import { Capsule, CapsuleError, type CapsuleItem, MAX_ITEMS } from '../src/capsule';
import { fromBase64url, toBase64url } from '../src/helpers/bytes';
import { KNOWN_LANGUAGES } from '../src/languages';
import { LinkError, parseLink, pasteIdFromInput } from '../src/links';
import { MAX_PBKDF2_ITERATIONS, MIN_PBKDF2_ITERATIONS, PublicHeader } from '../src/public_header';

const RUNS = { numRuns: 300 };

const presentation = fc.constantFrom('plain', 'code', 'markdown', 'download');

/** Names within the 255-byte UTF-8 limit (a code point is at most 4 bytes). */
const itemName = fc.string({ minLength: 1, maxLength: 60, unit: 'grapheme' });

const item = (index: number): fc.Arbitrary<CapsuleItem> =>
    fc
        .record({ name: itemName, bytes: fc.uint8Array({ maxLength: 512 }), presentation })
        .map(({ name, bytes, presentation }) => ({
            id: `item-${index}`,
            name,
            size: bytes.length,
            mediaType: 'application/octet-stream',
            presentation,
            bytes,
        }));

const items = fc
    .integer({ min: 1, max: MAX_ITEMS })
    .chain((count) => fc.tuple(...Array.from({ length: count }, (_, i) => item(i))));

/**
 * Runs `fn` and fails the property if it throws anything but `allowed`. A
 * `TypeError` or `RangeError` is always a failure — those mean the parser
 * touched something it had not checked (a property of `null`, an out-of-range
 * length), which is exactly what these tests look for.
 */
function onlyThrows(allowed: new (...args: never[]) => Error, fn: () => unknown): void {
    try {
        fn();
    } catch (err) {
        if (err instanceof TypeError || err instanceof RangeError || !(err instanceof allowed)) {
            throw new Error(`threw ${String(err)} instead of ${allowed.name}`);
        }
    }
}

describe('base64url', () => {
    it('round-trips any bytes', () => {
        fc.assert(
            fc.property(fc.uint8Array({ maxLength: 1024 }), (bytes) => {
                const text = toBase64url(bytes);
                expect(text).toMatch(/^[A-Za-z0-9_-]*$/);
                expect(Array.from(fromBase64url(text))).toEqual(Array.from(bytes));
            }),
            RUNS,
        );
    });
});

describe('Capsule', () => {
    it('round-trips any valid set of items', () => {
        fc.assert(
            fc.property(items, fc.option(fc.string({ maxLength: 80 })), (list, title) => {
                const decoded = Capsule.decode(new Capsule(list, title).encode());
                expect(decoded.title).toBe(title);
                expect(
                    decoded.items.map((i) => [i.name, i.presentation, Array.from(i.bytes)]),
                ).toEqual(list.map((i) => [i.name, i.presentation, Array.from(i.bytes)]));
            }),
            RUNS,
        );
    });

    it('rejects arbitrary bytes only with CapsuleError', () => {
        fc.assert(
            fc.property(fc.uint8Array({ maxLength: 256 }), (bytes) => {
                onlyThrows(CapsuleError, () => Capsule.decode(bytes));
            }),
            RUNS,
        );
    });

    it('rejects arbitrary bytes behind a valid header only with CapsuleError', () => {
        const header = [0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00, 0x00, 0x01];
        fc.assert(
            fc.property(fc.uint8Array({ maxLength: 512 }), (rest) => {
                onlyThrows(CapsuleError, () =>
                    Capsule.decode(new Uint8Array([...header, ...rest])),
                );
            }),
            RUNS,
        );
    });

    it('rejects any manifest JSON value it does not understand only with CapsuleError', () => {
        fc.assert(
            fc.property(fc.jsonValue(), fc.uint8Array({ maxLength: 64 }), (manifest, payload) => {
                const json = new TextEncoder().encode(JSON.stringify(manifest));
                const bytes = new Uint8Array(14 + json.length + payload.length);
                bytes.set([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00, 0x00, 0x01]);
                new DataView(bytes.buffer).setUint32(10, json.length);
                bytes.set(json, 14);
                bytes.set(payload, 14 + json.length);
                onlyThrows(CapsuleError, () => Capsule.decode(bytes));
            }),
            RUNS,
        );
    });

    it('detects any single corrupted or truncated byte of a valid capsule, or decodes cleanly', () => {
        fc.assert(
            fc.property(
                items,
                fc.nat(),
                fc.integer({ min: 1, max: 255 }),
                fc.boolean(),
                (list, position, flip, truncate) => {
                    const encoded = new Capsule(list).encode();
                    const at = position % encoded.length;
                    const damaged = truncate ? encoded.slice(0, at) : encoded.slice();
                    if (!truncate) damaged[at] = damaged[at]! ^ flip;
                    onlyThrows(CapsuleError, () => Capsule.decode(damaged));
                },
            ),
            RUNS,
        );
    });

    it('never trusts a declared manifest length beyond the data', () => {
        fc.assert(
            fc.property(fc.integer({ min: 0, max: 0xffffffff }), (declared) => {
                const bytes = new Uint8Array(14 + 8);
                bytes.set([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00, 0x00, 0x01]);
                new DataView(bytes.buffer).setUint32(10, declared);
                onlyThrows(CapsuleError, () => Capsule.decode(bytes));
            }),
            RUNS,
        );
    });
});

describe('PublicHeader', () => {
    const header = fc
        .record({
            nonce: fc.uint8Array({ minLength: 12, maxLength: 12 }),
            salt: fc.uint8Array({ minLength: 16, maxLength: 16 }),
            password: fc.option(
                fc.record({
                    salt: fc.uint8Array({ minLength: 16, maxLength: 16 }),
                    iterations: fc.integer({
                        min: MIN_PBKDF2_ITERATIONS,
                        max: MAX_PBKDF2_ITERATIONS,
                    }),
                }),
                { nil: undefined },
            ),
        })
        .map(
            ({ nonce, salt, password }) =>
                new PublicHeader(
                    nonce,
                    salt,
                    password && {
                        required: true,
                        kdf: 'PBKDF2-HMAC-SHA-256',
                        salt: toBase64url(password.salt),
                        iterations: password.iterations,
                    },
                ),
        );

    it('round-trips through its canonical AAD bytes', () => {
        fc.assert(
            fc.property(header, (h) => {
                const aad = h.toAad();
                const parsed = PublicHeader.parseFromJson(new TextDecoder().decode(aad));
                // Canonical: re-serialising gives the identical bytes.
                expect(Array.from(parsed.toAad())).toEqual(Array.from(aad));
            }),
            RUNS,
        );
    });

    it('refuses every integer iteration count below the floor', () => {
        fc.assert(
            fc.property(
                header.filter((h) => h.hasPassword()),
                fc.integer({ min: Number.MIN_SAFE_INTEGER, max: MIN_PBKDF2_ITERATIONS - 1 }),
                (h, iterations) => {
                    const weak = new PublicHeader(h.nonceBytes(), h.hkdfSaltBytes(), {
                        ...h.password!,
                        iterations,
                    });
                    const json = new TextDecoder().decode(weak.toAad());
                    expect(() => PublicHeader.parseFromJson(json)).toThrow(/iterations/);
                },
            ),
            RUNS,
        );
    });

    it('rejects arbitrary JSON and arbitrary strings with an Error, nothing else', () => {
        fc.assert(
            fc.property(fc.oneof(fc.string(), fc.json()), (text) => {
                onlyThrows(Error, () => PublicHeader.parseFromJson(text));
            }),
            RUNS,
        );
    });
});

describe('links', () => {
    it('reject arbitrary input only with LinkError', () => {
        fc.assert(
            fc.property(fc.oneof(fc.string(), fc.webUrl({ withFragments: true })), (text) => {
                onlyThrows(LinkError, () => parseLink(text, 'share'));
                onlyThrows(LinkError, () => parseLink(text, 'manage'));
            }),
            RUNS,
        );
    });

    it('never take a paste id from the fragment', () => {
        const id = fc.stringMatching(/^[A-Za-z0-9_-]{22}$/);
        fc.assert(
            fc.property(fc.webUrl(), id, (url, inFragment) => {
                const result = pasteIdFromInput(`${url.split('#')[0]}#${inFragment}`);
                if (result !== null) expect(result).not.toBe(inFragment);
            }),
            RUNS,
        );
    });

    it('find the id in any share link built from one', () => {
        const id = fc.stringMatching(/^[A-Za-z0-9_-]{22}$/);
        const secret = fc.uint8Array({ minLength: 32, maxLength: 32 });
        const subPath = fc.constantFrom('', '/sub', '/cb', '/a/b');
        const language = fc.constantFrom('', ...KNOWN_LANGUAGES.map((code) => `/${code}`));
        fc.assert(
            fc.property(id, secret, subPath, language, (pasteId, key, sub, lang) => {
                const base = `https://paste.example${sub}`;
                const link = `${base}${lang}/p/${pasteId}#v1.${toBase64url(key)}`;
                expect(pasteIdFromInput(link)).toBe(pasteId);
                const parsed = parseLink(link, 'share');
                expect(parsed.id).toBe(pasteId);
                expect(parsed.base).toBe(base);
                expect(Array.from(parsed.secret)).toEqual(Array.from(key));
            }),
            RUNS,
        );
    });
});
