/**
 * The API client bounds what it reads from a server: every response body,
 * error bodies included, is read as a stream with a per-endpoint byte limit,
 * and every request has a deadline. A hostile or broken server can neither
 * exhaust the client's memory nor hold a call open.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, MAX_CIPHERTEXT_BYTES, PasteApi, RESPONSE_TOO_LARGE, TIMEOUT } from '../src/api';
import { MAX_ITEM_BYTES } from '../src/capsule';
import { fromBase64url } from '../src/helpers/bytes';
import { parseLink } from '../src/links';
import { openPaste } from '../src/open';
import { Paste } from '../src/paste';
import { createFakeServer } from './fake-server';
import { CHUNK, endlessBody, observe, sealedPaste, stallingBody } from './limits-support';

const ORIGIN = 'http://capsule.test';
const ID = 'A'.repeat(22);
const SMALL_LIMIT = 64 * 1024;
/** Unpadded base64url of the largest ciphertext plus the JSON allowance. */
const RETRIEVE_LIMIT = Math.ceil((MAX_CIPHERTEXT_BYTES * 4) / 3) + 1024;

afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

const calls: Record<string, (api: PasteApi) => Promise<unknown>> = {
    create: async (api) => api.create(await sealedPaste()),
    describe: (api) => api.getDescriptor(ID, { describeToken: new Uint8Array(32) }),
    retrieve: (api) => api.retrieve(ID, { describeToken: 'AA', revealToken: 'AA' }),
    delete: (api) => api.delete(ID, 'AA'),
};

describe('response limits', () => {
    const cases: [string, number, number][] = [
        ['create', 201, SMALL_LIMIT],
        ['describe', 200, SMALL_LIMIT],
        ['retrieve', 200, RETRIEVE_LIMIT],
        ['delete', 200, SMALL_LIMIT],
    ];
    for (const name of Object.keys(calls)) {
        cases.push([name, 404, SMALL_LIMIT], [name, 500, SMALL_LIMIT]);
    }
    const lengths: [string, (size: number) => string | null][] = [
        ['without Content-Length', () => null],
        ['with a small wrong Content-Length', () => '2'],
        ['with a correct Content-Length', (size) => String(size)],
    ];

    it('oversized responses are cut off', async () => {
        for (const [name, status, limit] of cases) {
            for (const [variant, contentLength] of lengths) {
                const size = limit + 4 * CHUNK;
                const body = endlessBody(size);
                vi.stubGlobal('fetch', async () => {
                    const headers = new Headers({ 'Content-Type': 'application/json' });
                    const length = contentLength(size);
                    if (length !== null) headers.set('Content-Length', length);
                    return new Response(body.stream, { status, headers });
                });

                const error = await calls[name]!(new PasteApi(ORIGIN)).then(
                    () => undefined,
                    (err: unknown) => err,
                );

                const label = `${name} ${status} ${variant}`;
                expect(error, label).toBeInstanceOf(ApiError);
                expect(error, label).toMatchObject({ status: 0, code: RESPONSE_TOO_LARGE });
                expect(body.state.cancelled, label).toBe(true);
                expect(body.state.read, label).toBeLessThanOrEqual(limit + CHUNK);
            }
        }
    });
});

describe('responses within the limits', () => {
    it('a capsule at the size limit still opens', async () => {
        const server = createFakeServer(ORIGIN);
        vi.stubGlobal('fetch', server.fetch);

        const paste = new Paste({ expiresIn: 3600, burnAfterRetrieval: false });
        const bytes = new Uint8Array(MAX_ITEM_BYTES).fill(0x61);
        paste.addItem({
            id: '01J00000000000000000000000',
            name: 'big.txt',
            size: bytes.length,
            mediaType: 'text/plain',
            presentation: 'plain',
            bytes,
        });
        const { shareUrl } = await paste.publish(new PasteApi(ORIGIN), ORIGIN);
        const share = parseLink(shareUrl, 'share');
        // Largest padding: the stored ciphertext is a full-size capsule plus the tag.
        const stored = server.pastes.get(share.id)!;
        expect(fromBase64url(stored.ciphertext!).length).toBe(MAX_CIPHERTEXT_BYTES);

        const outcome = await openPaste(new PasteApi(share.base), share.id, share.secret);
        expect(outcome.kind).toBe('success');
        if (outcome.kind !== 'success') return;
        expect(outcome.capsule.items[0]!.bytes.length).toBe(MAX_ITEM_BYTES);

        vi.stubGlobal(
            'fetch',
            async () =>
                new Response(JSON.stringify({ title: 'Not available', code: 'not_available' }), {
                    status: 404,
                    headers: { 'Retry-After': '5' },
                }),
        );
        await expect(calls['describe']!(new PasteApi(ORIGIN))).rejects.toMatchObject({
            status: 404,
            code: 'not_available',
            retryAfter: 5,
        });
    }, 60_000);
});

describe('deadlines', () => {
    /** A `fetch` whose response headers never arrive; it rejects when aborted, like the real one. */
    function hangingFetch() {
        const seen: { signal: AbortSignal | null } = { signal: null };
        vi.stubGlobal('fetch', (_url: string, init?: RequestInit) => {
            seen.signal = init?.signal ?? null;
            return new Promise<Response>((_, reject) => {
                seen.signal?.addEventListener('abort', () => reject(new Error('aborted')));
            });
        });
        return seen;
    }

    /** A `fetch` answering at once with `body`, remembering the request's signal. */
    function answeringFetch(body: ReadableStream<Uint8Array>) {
        const seen: { signal: AbortSignal | null } = { signal: null };
        vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) => {
            seen.signal = init?.signal ?? null;
            return new Response(body, { status: 200 });
        });
        return seen;
    }

    async function expectTimeoutAt(
        call: Promise<unknown>,
        ms: number,
        signal: () => AbortSignal | null,
        /** Timers the test's own stream still has pending; the client must leave none. */
        foreignTimers = 0,
    ) {
        const state = observe(call);
        await vi.advanceTimersByTimeAsync(ms - 1);
        expect(state.settled).toBe(false);
        await vi.advanceTimersByTimeAsync(1);
        expect(state.settled).toBe(true);
        expect(state.error).toBeInstanceOf(ApiError);
        expect(state.error).toMatchObject({ status: 0, code: TIMEOUT });
        expect(signal()?.aborted).toBe(true);
        expect(vi.getTimerCount()).toBe(foreignTimers);
    }

    it('requests time out', async () => {
        vi.useFakeTimers();

        // Headers that never arrive: 30 s for describe and delete.
        for (const name of ['describe', 'delete']) {
            const seen = hangingFetch();
            await expectTimeoutAt(calls[name]!(new PasteApi(ORIGIN)), 30_000, () => seen.signal);
        }

        // A retrieve body trickling one byte every 29 s runs until the 10 min deadline.
        let trickled = false;
        const trickle = new ReadableStream<Uint8Array>(
            {
                async pull(controller) {
                    await new Promise((resolve) => setTimeout(resolve, 29_000));
                    controller.enqueue(new Uint8Array([0x20]));
                },
                cancel() {
                    trickled = true;
                },
            },
            { highWaterMark: 0 },
        );
        let seen = answeringFetch(trickle);
        await expectTimeoutAt(
            calls['retrieve']!(new PasteApi(ORIGIN)),
            600_000,
            () => seen.signal,
            1,
        );
        await vi.runAllTimersAsync();
        expect(trickled).toBe(true);

        // Any body that stalls for 30 s, well before the total deadline.
        const stall = stallingBody();
        seen = answeringFetch(stall.stream);
        await expectTimeoutAt(calls['retrieve']!(new PasteApi(ORIGIN)), 30_000, () => seen.signal);
        expect(stall.state.cancelled).toBe(true);
    });
});
