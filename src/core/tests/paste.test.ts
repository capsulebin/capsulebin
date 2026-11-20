/**
 * Tests for the paste domain module: key derivation, verifiers, and the
 * `Paste` builder that turns items and options into a create request.
 */

import { describe, expect, it, vi } from 'vitest';
import {
    deleteVerifier,
    derivePasteKeys,
    Paste,
    revealVerifier,
    type SealedPaste,
} from '../src/paste';
import { PasteApi, type CreateResponse } from '../src/api';
import { decrypt, derivePasswordKey, randomBytes, sha256 } from '../src/crypto';
import { fromBase64url, toBase64url } from '../src/helpers/bytes';
import { Capsule, CapsuleError, type CapsuleItem } from '../src/capsule';
import { PublicHeader } from '../src/public_header';

/** Converts a Uint8Array to a plain array for deep-equal comparison. */
function toArray(bytes: Uint8Array): number[] {
    return Array.from(bytes);
}

describe('derivePasteKeys', () => {
    it('derives a content key that can be exported', async () => {
        const linkSecret = randomBytes(32);
        const hkdfSalt = randomBytes(16);
        const { contentKey } = await derivePasteKeys(linkSecret, hkdfSalt);
        const raw = await crypto.subtle.exportKey('raw', contentKey);
        expect(new Uint8Array(raw)).toHaveLength(32);
    });

    it('derives a 32-byte reveal token', async () => {
        const linkSecret = randomBytes(32);
        const hkdfSalt = randomBytes(16);
        const { revealToken } = await derivePasteKeys(linkSecret, hkdfSalt);
        expect(revealToken).toHaveLength(32);
    });

    it('derives distinct content key and reveal token from the same link secret', async () => {
        const linkSecret = randomBytes(32);
        const hkdfSalt = randomBytes(16);

        const { contentKey, revealToken } = await derivePasteKeys(linkSecret, hkdfSalt);

        const contentBytes = new Uint8Array(await crypto.subtle.exportKey('raw', contentKey));
        expect(toArray(contentBytes)).not.toEqual(toArray(revealToken));
    });

    it('mixes a password key into both derivations', async () => {
        const linkSecret = new Uint8Array(32).fill(0x42);
        const hkdfSalt = new Uint8Array(16).fill(0x13);
        const passwordKey = new Uint8Array(32).fill(0x99);

        const withoutPassword = await derivePasteKeys(linkSecret, hkdfSalt);
        const withPassword = await derivePasteKeys(linkSecret, hkdfSalt, passwordKey);
        expect(withPassword.revealToken).not.toEqual(withoutPassword.revealToken);
    });
});

describe('verifiers', () => {
    it('derives a reveal verifier of 32 bytes', async () => {
        const token = new Uint8Array(32).fill(0xaa);
        const verifier = await revealVerifier(token);
        expect(verifier).toHaveLength(32);
    });

    it('derives a delete verifier of 32 bytes, domain-separated from raw sha256', async () => {
        const secret = new Uint8Array(32).fill(0xbb);
        const verifier = await deleteVerifier(secret);
        const raw = await sha256(secret);
        expect(verifier).toHaveLength(32);
        expect(verifier).not.toEqual(raw);
    });
});

const item: CapsuleItem = {
    id: '01J00000000000000000000001',
    name: 'hello.txt',
    size: 5,
    mediaType: 'text/plain',
    presentation: 'plain',
    bytes: new TextEncoder().encode('hello'),
};

function headerOf(sealed: SealedPaste): PublicHeader {
    return PublicHeader.parseFromJson(new TextDecoder().decode(fromBase64url(sealed.publicHeader)));
}

describe('Paste.addItem', () => {
    it('rejects a 17th item', () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        for (let i = 0; i < 16; i++) {
            paste.addItem({ ...item, id: `${item.id}-${i}` });
        }
        expect(paste.items).toHaveLength(16);
        expect(() => paste.addItem({ ...item, id: 'one-too-many' })).toThrow(CapsuleError);
    });

    it('rejects adding items after the paste has been sealed', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        await paste.seal();
        expect(() => paste.addItem(item)).toThrow(/sealed/i);
    });
});

describe('Paste.seal', () => {
    it('is one-shot: sealing twice throws', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        await paste.seal();
        await expect(paste.seal()).rejects.toThrow(/already been sealed/i);
    });

    it('generates 32-byte, independent link and delete secrets', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();
        expect(sealed.linkSecret).toHaveLength(32);
        expect(sealed.deleteSecret).toHaveLength(32);
        expect(sealed.linkSecret).not.toEqual(sealed.deleteSecret);
        expect(paste.sealed).toBe(sealed);
    });

    it('produces a sealed paste the recipient can decrypt with the returned link secret', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();

        const header = headerOf(sealed);
        expect(header.hasPassword()).toBe(false);

        const { contentKey } = await derivePasteKeys(sealed.linkSecret, header.hkdfSaltBytes());
        const plaintext = await decrypt(
            contentKey,
            header.nonceBytes(),
            fromBase64url(sealed.ciphertext),
            fromBase64url(sealed.publicHeader),
        );
        const capsule = Capsule.decode(plaintext);
        expect(capsule.items).toHaveLength(1);
        expect(capsule.items[0]!.name).toBe('hello.txt');
    });

    it('the correct reveal token matches the stored verifier, a wrong one does not', async () => {
        const paste = new Paste({ password: 'sesame', expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();

        const header = headerOf(sealed);
        expect(header.hasPassword()).toBe(true);
        const hkdfSalt = header.hkdfSaltBytes();

        const { revealToken: rightToken } = await derivePasteKeys(
            sealed.linkSecret,
            hkdfSalt,
            await derivePasswordKey('sesame', fromBase64url(header.password!.salt)),
        );
        expect(toBase64url(await revealVerifier(rightToken))).toBe(sealed.revealVerifier);

        // Deriving without the password produces a different token, and thus a
        // verifier the server would reject.
        const { revealToken: wrongToken } = await derivePasteKeys(sealed.linkSecret, hkdfSalt);
        expect(toBase64url(await revealVerifier(wrongToken))).not.toBe(sealed.revealVerifier);
    });

    it('refuses a non-empty password of only whitespace before anything is sent', async () => {
        for (const password of [' ', '   ', '\t\n']) {
            const paste = new Paste({ password, expiresIn: null, burnAfterRetrieval: false });
            paste.addItem(item);
            await expect(paste.seal()).rejects.toThrow(/whitespace/);
            expect(paste.sealed).toBeUndefined();

            const api = new PasteApi('https://example.test');
            const createSpy = vi.spyOn(api, 'create');
            await expect(paste.publish(api, 'https://example.test')).rejects.toThrow(/whitespace/);
            expect(createSpy).not.toHaveBeenCalled();
        }
    });

    it('treats an empty or null password as no password', async () => {
        for (const password of ['', null]) {
            const paste = new Paste({ password, expiresIn: null, burnAfterRetrieval: false });
            paste.addItem(item);
            const sealed = await paste.seal();
            expect(headerOf(sealed).hasPassword()).toBe(false);
        }
    });

    it('carries expiry and burn-after-retrieval through to the sealed paste', async () => {
        const paste = new Paste({ expiresIn: 3600, burnAfterRetrieval: true });
        paste.addItem(item);
        const sealed = await paste.seal();
        expect(sealed.expiresIn).toBe(3600);
        expect(sealed.burnAfterRetrieval).toBe(true);
    });
});

describe('Paste share / delete URLs', () => {
    it('throws when called before seal', () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        expect(() => paste.shareUrl('https://example.test')).toThrow(/not been sealed/i);
        expect(() => paste.deleteUrl('https://example.test')).toThrow(/not been sealed/i);
    });

    it('embeds the fragment-prefixed link secret in the share URL', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();

        const url = paste.shareUrl('https://example.test');
        expect(url).toBe(
            `https://example.test/p/${sealed.id}#v1.${toBase64url(sealed.linkSecret)}`,
        );
    });

    it('embeds the fragment-prefixed delete secret in the delete URL, under /manage/', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();

        const url = paste.deleteUrl('https://example.test');
        expect(url).toBe(
            `https://example.test/manage/${sealed.id}#v1.${toBase64url(sealed.deleteSecret)}`,
        );
    });
});

describe('Paste.publish', () => {
    it('seals, calls api.create once with the sealed paste, and returns the matching URLs', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);

        const api = new PasteApi('https://example.test');
        const createSpy = vi.spyOn(api, 'create').mockResolvedValue({
            id: 'server-would-echo-the-same-id',
            expires_at: null,
            burn_after_retrieval: false,
        } as CreateResponse);

        const { shareUrl, deleteUrl } = await paste.publish(api, 'https://example.test');

        expect(createSpy).toHaveBeenCalledTimes(1);
        expect(createSpy).toHaveBeenCalledWith(paste.sealed);
        expect(shareUrl).toBe(paste.shareUrl('https://example.test'));
        expect(deleteUrl).toBe(paste.deleteUrl('https://example.test'));
    });

    it('does not re-seal if the paste was already sealed', async () => {
        const paste = new Paste({ expiresIn: null, burnAfterRetrieval: false });
        paste.addItem(item);
        const sealed = await paste.seal();

        const api = new PasteApi('https://example.test');
        vi.spyOn(api, 'create').mockResolvedValue({
            id: sealed.id,
            expires_at: null,
            burn_after_retrieval: false,
        } as CreateResponse);

        await paste.publish(api, 'https://example.test');
        expect(paste.sealed).toBe(sealed);
    });
});
