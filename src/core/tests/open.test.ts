/**
 * The recipient flow (`openPaste`) end to end against an in-memory server:
 * publish with `Paste`, open with the share link, and check that every
 * failure ends in the coarse outcome the clients rely on.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, PasteApi } from '../src/api';
import { encodeUtf8, decodeUtf8, fromBase64url, toBase64url } from '../src/helpers/bytes';
import { parseLink } from '../src/links';
import { derivePasswordKeyFor, openPaste } from '../src/open';
import { Paste, type PasteOptions } from '../src/paste';
import { createFakeServer, type FakeServer } from './fake-server';

const ORIGIN = 'http://capsule.test';
let server: FakeServer;

beforeEach(() => {
    server = createFakeServer(ORIGIN);
    vi.stubGlobal('fetch', server.fetch);
});

afterEach(() => {
    vi.unstubAllGlobals();
});

async function publish(options: Partial<PasteOptions> = {}, text = 'hello capsule') {
    const paste = new Paste({ expiresIn: 3600, burnAfterRetrieval: false, ...options });
    const bytes = encodeUtf8(text);
    paste.addItem({
        id: '01J00000000000000000000000',
        name: 'note.md',
        size: bytes.length,
        mediaType: 'text/markdown',
        presentation: 'markdown',
        bytes,
    });
    const { shareUrl, deleteUrl } = await paste.publish(new PasteApi(ORIGIN), ORIGIN);
    return { share: parseLink(shareUrl, 'share'), manage: parseLink(deleteUrl, 'manage') };
}

describe('openPaste', () => {
    it('opens a published capsule with its share link', async () => {
        const { share } = await publish();
        const outcome = await openPaste(new PasteApi(share.base), share.id, share.secret);

        expect(outcome.kind).toBe('success');
        if (outcome.kind !== 'success') return;
        expect(outcome.capsule.items).toHaveLength(1);
        expect(decodeUtf8(outcome.capsule.items[0]!.bytes)).toBe('hello capsule');
        expect(outcome.burnAfterRetrieval).toBe(false);
    });

    it('never sends the link secret over the wire', async () => {
        const { share } = await publish();
        await openPaste(new PasteApi(share.base), share.id, share.secret);

        const secret = share.secret;
        const b64 = Buffer.from(secret).toString('base64url');
        for (const request of server.requests) {
            expect(request.url).not.toContain(b64);
            expect(request.body).not.toContain(b64);
        }
    });

    it('reports a wrong link secret as the generic api error, not as a decrypt failure', async () => {
        const { share } = await publish();
        const wrong = new Uint8Array(32).fill(7);
        const outcome = await openPaste(new PasteApi(share.base), share.id, wrong);

        expect(outcome.kind).toBe('api_error');
        if (outcome.kind === 'api_error') expect(outcome.error.status).toBe(404);
    });

    it('reports an unknown id exactly like a wrong secret', async () => {
        const { share } = await publish();
        const outcome = await openPaste(new PasteApi(share.base), 'A'.repeat(22), share.secret);
        expect(outcome).toEqual({
            kind: 'api_error',
            error: new ApiError('not_available', 404, 'not_available'),
        });
    });

    it('needs a passwordKey hook for a password-protected capsule', async () => {
        const { share } = await publish({ password: 'correct horse' });
        const outcome = await openPaste(new PasteApi(share.base), share.id, share.secret);
        expect(outcome.kind).toBe('password_required');
        // Nothing was retrieved.
        expect(server.requests.filter((r) => r.url.endsWith('/retrieve'))).toHaveLength(0);
    });

    it('ends as invalid_capsule on a header below the KDF floor, before any password or retrieve', async () => {
        const { share } = await publish({ password: 'correct horse' });
        const stored = server.pastes.get(share.id)!;
        const json = decodeUtf8(fromBase64url(stored.publicHeader));
        const weak = json.replace(/"iterations":\d+/, '"iterations":99999');
        expect(weak).not.toBe(json);
        stored.publicHeader = toBase64url(encodeUtf8(weak));
        const passwordKey = vi.fn((header) => derivePasswordKeyFor(header, 'correct horse'));
        server.requests.length = 0;

        const outcome = await openPaste(new PasteApi(share.base), share.id, share.secret, {
            passwordKey,
        });

        expect(outcome.kind).toBe('invalid_capsule');
        expect(passwordKey).not.toHaveBeenCalled();
        expect(server.requests.map((r) => new URL(r.url).pathname)).toEqual([
            `/api/v1/pastes/${share.id}/describe`,
        ]);
    });

    it('opens with the right password and is refused with a wrong one', async () => {
        const { share } = await publish({ password: 'correct horse' });
        const api = new PasteApi(share.base);

        const wrong = await openPaste(api, share.id, share.secret, {
            passwordKey: (header) => derivePasswordKeyFor(header, 'battery staple'),
        });
        expect(wrong.kind).toBe('api_error');

        const right = await openPaste(api, share.id, share.secret, {
            passwordKey: (header) => derivePasswordKeyFor(header, 'correct horse'),
        });
        expect(right.kind).toBe('success');
    });

    it('hands a burn capsule out once; afterwards it is simply not available', async () => {
        const { share } = await publish({ burnAfterRetrieval: true });
        const api = new PasteApi(share.base);

        const first = await openPaste(api, share.id, share.secret);
        expect(first.kind).toBe('success');
        if (first.kind === 'success') expect(first.burnAfterRetrieval).toBe(true);

        // Like the real server: a consumed capsule is indistinguishable from a
        // missing one.
        const second = await openPaste(api, share.id, share.secret);
        expect(second.kind).toBe('api_error');
        if (second.kind === 'api_error') expect(second.error.status).toBe(404);
    });

    it('reports a lost burn race (410) as burned', async () => {
        const { share } = await publish({ burnAfterRetrieval: true });
        server.failNextBurnWithRace = true;
        expect((await openPaste(new PasteApi(share.base), share.id, share.secret)).kind).toBe(
            'burned',
        );
    });

    it('does not consume a burn capsule when confirmBurn rejects', async () => {
        const { share } = await publish({ burnAfterRetrieval: true });
        const api = new PasteApi(share.base);

        const declined = await openPaste(api, share.id, share.secret, {
            confirmBurn: () => Promise.reject(new Error('no')),
        });
        expect(declined.kind).toBe('aborted');

        expect((await openPaste(api, share.id, share.secret)).kind).toBe('success');
    });

    it('sends no retrieve request when beforeRetrieve stops', async () => {
        const { share } = await publish({ burnAfterRetrieval: true });
        const api = new PasteApi(share.base);
        const retrieves = () => server.requests.filter((r) => r.url.endsWith('/retrieve'));

        const stopped = await openPaste(api, share.id, share.secret, {
            beforeRetrieve: () => false,
        });
        expect(stopped.kind).toBe('aborted');
        expect(retrieves()).toHaveLength(0);

        let asked = 0;
        const opened = await openPaste(api, share.id, share.secret, {
            beforeRetrieve: () => {
                asked += 1;
                return true;
            },
        });
        expect(opened.kind).toBe('success');
        expect(asked).toBe(1);
        expect(retrieves()).toHaveLength(1);
    });

    it('reports a tampered ciphertext as decrypt_failed', async () => {
        const { share } = await publish();
        const stored = server.pastes.get(share.id)!;
        const bytes = Buffer.from(stored.ciphertext!, 'base64url');
        bytes[0]! ^= 0xff;
        stored.ciphertext = bytes.toString('base64url');

        const outcome = await openPaste(new PasteApi(share.base), share.id, share.secret);
        expect(outcome.kind).toBe('decrypt_failed');
    });
});

describe('PasteApi creation token', () => {
    it('sends the bearer on create only', async () => {
        server.creationToken = 'cbt_secret';
        const paste = new Paste({ expiresIn: 60, burnAfterRetrieval: false });
        const bytes = encodeUtf8('x');
        paste.addItem({
            id: '01J00000000000000000000000',
            name: 'x.txt',
            size: 1,
            mediaType: 'text/plain',
            presentation: 'plain',
            bytes,
        });
        const api = new PasteApi(ORIGIN, { creationToken: 'cbt_secret' });
        const { shareUrl } = await paste.publish(api, ORIGIN);
        const share = parseLink(shareUrl, 'share');
        await openPaste(api, share.id, share.secret);

        const [create, ...rest] = server.requests;
        expect(create!.headers.get('Authorization')).toBe('Bearer cbt_secret');
        for (const request of rest) expect(request.headers.get('Authorization')).toBeNull();
    });

    it('is rejected without the token', async () => {
        server.creationToken = 'cbt_secret';
        const paste = new Paste({ expiresIn: 60, burnAfterRetrieval: false });
        paste.addItem({
            id: '01J00000000000000000000000',
            name: 'x.txt',
            size: 1,
            mediaType: 'text/plain',
            presentation: 'plain',
            bytes: encodeUtf8('x'),
        });
        await expect(paste.publish(new PasteApi(ORIGIN), ORIGIN)).rejects.toMatchObject({
            status: 401,
            code: 'unauthorized',
        });
    });
});

describe('PasteApi errors', () => {
    it('carries Retry-After from a 429', async () => {
        vi.stubGlobal('fetch', async () => {
            return new Response(
                JSON.stringify({ title: 'Too many requests', code: 'rate_limited' }),
                {
                    status: 429,
                    headers: { 'Retry-After': '17' },
                },
            );
        });
        await expect(
            new PasteApi(ORIGIN).getDescriptor('A'.repeat(22), {
                describeToken: new Uint8Array(32),
            }),
        ).rejects.toMatchObject({
            status: 429,
            code: 'rate_limited',
            retryAfter: 17,
        });
    });

    it('turns a non-JSON error body into an ApiError instead of returning it', async () => {
        vi.stubGlobal('fetch', async () => new Response('"oops"', { status: 500 }));
        await expect(
            new PasteApi(ORIGIN).getDescriptor('A'.repeat(22), {
                describeToken: new Uint8Array(32),
            }),
        ).rejects.toMatchObject({
            status: 500,
            code: null,
        });
    });
});
