/**
 * Golden vector test: verifies this implementation against the language-neutral
 * vectors in `docs/test-vectors/capsule-v1.json`, so a second implementation
 * (or a future one in another language) can be checked the same way.
 */

import { describe, expect, it } from 'vitest';
import { Capsule, CapsuleError, type CapsuleItem } from '../src/capsule';
import { decrypt, derivePasswordKey, encrypt, sha256 } from '../src/crypto';
import { fromBase64url, toBase64url } from '../src/helpers/bytes';
import { deleteVerifier, deriveDescribeToken, derivePasteKeys, revealVerifier } from '../src/paste';

import vectors from '../../../docs/test-vectors/capsule-v1.json' with { type: 'json' };

function hexToBytes(hex: string): Uint8Array {
    const bytes = new Uint8Array(hex.length / 2);
    for (let i = 0; i < hex.length; i += 2) {
        bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
    }
    return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
    return Array.from(bytes)
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('');
}

function toArray(bytes: Uint8Array): number[] {
    return Array.from(bytes);
}

describe('golden vectors', () => {
    const inputs = vectors.inputs;
    const expected = vectors.expected;

    const linkSecret = hexToBytes(inputs.link_secret_hex);
    const hkdfSalt = hexToBytes(inputs.hkdf_salt_hex);
    const nonce = hexToBytes(inputs.nonce_hex);
    const deleteSecret = hexToBytes(inputs.delete_secret_hex);
    const headerBytes = new TextEncoder().encode(inputs.public_header_json);

    it('content key matches reference', async () => {
        const { contentKey } = await derivePasteKeys(linkSecret, hkdfSalt);
        const raw = new Uint8Array(await crypto.subtle.exportKey('raw', contentKey));
        expect(bytesToHex(raw)).toBe(expected.content_key_hex);
    });

    it('reveal token matches reference', async () => {
        const { revealToken } = await derivePasteKeys(linkSecret, hkdfSalt);
        expect(bytesToHex(revealToken)).toBe(expected.reveal_token_hex);
    });

    it('reveal verifier matches reference', async () => {
        const token = hexToBytes(expected.reveal_token_hex);
        const verifier = await revealVerifier(token);
        expect(toBase64url(verifier)).toBe(expected.reveal_verifier_b64);
    });

    it('delete verifier matches reference', async () => {
        const verifier = await deleteVerifier(deleteSecret);
        expect(toBase64url(verifier)).toBe(expected.delete_verifier_b64);
    });

    it('capsule bytes and ciphertext match reference', async () => {
        const textBytes = new TextEncoder().encode(inputs.item_text);
        const item: CapsuleItem = {
            id: '01J00000000000000000000001',
            name: 'hello.txt',
            size: textBytes.length,
            mediaType: 'text/plain',
            presentation: 'plain',
            bytes: textBytes,
        };
        const capsuleBytes = new Capsule([item]).encode();

        // Pinned assertion: capsule byte layout must not change.
        expect(toBase64url(capsuleBytes)).toBe(expected.capsule_b64);

        const { contentKey } = await derivePasteKeys(linkSecret, hkdfSalt);
        const ciphertext = await encrypt(contentKey, nonce, capsuleBytes, headerBytes);

        // Pinned assertion: ciphertext must match the reference.
        expect(toBase64url(ciphertext)).toBe(expected.ciphertext_b64);

        // Round-trip sanity check.
        const decrypted = await decrypt(contentKey, nonce, ciphertext, headerBytes);
        expect(toArray(decrypted)).toEqual(toArray(capsuleBytes));
    });

    // --- forward compat -------------------------------------------------------

    it('accepts a capsule with an unknown optional manifest field', () => {
        const fwd = vectors.forward_compat as { capsule_b64: string };
        const bytes = fromBase64url(fwd.capsule_b64);
        const decoded = Capsule.decode(bytes);
        expect(decoded.items).toHaveLength(1);
        expect(decoded.items[0]!.presentation).toBe('plain');
    });

    // --- password vectors ----------------------------------------------------

    const passwordVectors = vectors.password as
        | {
              inputs: {
                  link_secret_hex: string;
                  hkdf_salt_hex: string;
                  password: string;
                  password_salt_hex: string;
                  password_iterations: number;
              };
              expected: {
                  password_key_hex: string;
                  content_key_hex: string;
                  reveal_token_hex: string;
              };
          }
        | undefined;

    if (passwordVectors) {
        const pwInputs = passwordVectors.inputs;
        const pwExpected = passwordVectors.expected;
        const linkSecret = hexToBytes(pwInputs.link_secret_hex);
        const hkdfSalt = hexToBytes(pwInputs.hkdf_salt_hex);
        const passwordSalt = hexToBytes(pwInputs.password_salt_hex);

        it('derives the correct password key via PBKDF2', async () => {
            const key = await derivePasswordKey(
                pwInputs.password,
                passwordSalt,
                pwInputs.password_iterations,
            );
            expect(bytesToHex(key)).toBe(pwExpected.password_key_hex);
        });

        it('derives different content key with password mixed into HKDF', async () => {
            const pwdKey = await derivePasswordKey(
                pwInputs.password,
                passwordSalt,
                pwInputs.password_iterations,
            );
            const { contentKey: ck } = await derivePasteKeys(linkSecret, hkdfSalt, pwdKey);
            const ckRaw = new Uint8Array(await crypto.subtle.exportKey('raw', ck));
            expect(bytesToHex(ckRaw)).toBe(pwExpected.content_key_hex);

            // Must differ from the no-password case.
            const { contentKey: ckNoPwd } = await derivePasteKeys(linkSecret, hkdfSalt);
            const ckNoPwdRaw = new Uint8Array(await crypto.subtle.exportKey('raw', ckNoPwd));
            expect(ckNoPwdRaw).not.toEqual(ckRaw);
        });

        it('derives different reveal token with password mixed into HKDF', async () => {
            const pwdKey = await derivePasswordKey(
                pwInputs.password,
                passwordSalt,
                pwInputs.password_iterations,
            );
            const { revealToken: rt } = await derivePasteKeys(linkSecret, hkdfSalt, pwdKey);
            expect(bytesToHex(rt)).toBe(pwExpected.reveal_token_hex);
        });
    }

    // --- negative vectors -----------------------------------------------------

    const negatives = vectors.negative as Array<{
        name: string;
        capsule_b64: string;
        error_contains: string;
    }>;

    for (const neg of negatives) {
        it(`rejects ${neg.name}`, () => {
            const bytes = fromBase64url(neg.capsule_b64);
            expect(() => Capsule.decode(bytes)).toThrow(CapsuleError);
            try {
                Capsule.decode(bytes);
            } catch (err) {
                expect((err as CapsuleError).message.toLowerCase()).toContain(neg.error_contains);
            }
        });
    }

    // --- describe token -------------------------------------------------------

    it('describe token and verifier match reference', async () => {
        const describe = vectors.describe;
        const token = await deriveDescribeToken(linkSecret, describe.inputs.paste_id);
        expect(bytesToHex(token)).toBe(describe.expected.describe_token_hex);
        expect(toBase64url(await sha256(token))).toBe(describe.expected.describe_verifier_b64);
    });

    // --- password normalization ----------------------------------------------

    it('normalizes passwords to NFC before PBKDF2', async () => {
        const nfc = vectors.password_nfc;
        const salt = hexToBytes(vectors.password.inputs.password_salt_hex);
        const iterations = vectors.password.inputs.password_iterations;
        expect(nfc.inputs.password_composed).not.toBe(nfc.inputs.password_decomposed);
        for (const password of [nfc.inputs.password_composed, nfc.inputs.password_decomposed]) {
            const key = await derivePasswordKey(password, salt, iterations);
            expect(bytesToHex(key)).toBe(nfc.expected.password_key_hex);
        }
    });

    it('pads the reference capsule to its PADMÉ length', () => {
        expect(fromBase64url(expected.capsule_b64)).toHaveLength(expected.capsule_padded_length);
        expect(expected.capsule_padded_length).toBeGreaterThan(expected.capsule_unpadded_length);
    });
});
