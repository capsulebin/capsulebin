/**
 * Acceptance edges of the client's read bounds (AC1–AC3): a body exactly at
 * the small limit is still read, the create deadline is the long one, and the
 * idle deadline applies to every endpoint, not only to retrieve.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, PasteApi, RESPONSE_TOO_LARGE, TIMEOUT } from '../src/api';
import { observe, sealedPaste, stallingBody } from './limits-support';

const ORIGIN = 'http://capsule.test';
const ID = 'A'.repeat(22);
const SMALL_LIMIT = 64 * 1024;

afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

/** A problem body padded with trailing whitespace to exactly `size` bytes. */
function problemOfSize(size: number): string {
    const json = JSON.stringify({ title: 'Not available', code: 'not_available' });
    return json + ' '.repeat(size - json.length);
}

describe('read bounds at the edges', () => {
    it('an error body of exactly 64 KiB is still read', async () => {
        vi.stubGlobal(
            'fetch',
            async () => new Response(problemOfSize(SMALL_LIMIT), { status: 404 }),
        );
        await expect(
            new PasteApi(ORIGIN).getDescriptor(ID, { describeToken: new Uint8Array(32) }),
        ).rejects.toMatchObject({ status: 404, code: 'not_available' });
    });

    it('an error body one byte above 64 KiB is cut off', async () => {
        vi.stubGlobal(
            'fetch',
            async () => new Response(problemOfSize(SMALL_LIMIT + 1), { status: 404 }),
        );
        await expect(
            new PasteApi(ORIGIN).getDescriptor(ID, { describeToken: new Uint8Array(32) }),
        ).rejects.toMatchObject({ status: 0, code: RESPONSE_TOO_LARGE });
    });

    it('a create waits up to 10 minutes for its answer, not 30 seconds', async () => {
        const sealed = await sealedPaste();
        vi.useFakeTimers();
        const seen: { signal: AbortSignal | null } = { signal: null };
        vi.stubGlobal('fetch', (_url: string, init?: RequestInit) => {
            seen.signal = init?.signal ?? null;
            return new Promise<Response>((_, reject) => {
                seen.signal?.addEventListener('abort', () => reject(new Error('aborted')));
            });
        });

        const state = observe(new PasteApi(ORIGIN).create(sealed));
        await vi.advanceTimersByTimeAsync(30_000);
        expect(state.settled).toBe(false);
        await vi.advanceTimersByTimeAsync(600_000 - 30_000 - 1);
        expect(state.settled).toBe(false);
        await vi.advanceTimersByTimeAsync(1);
        expect(state.settled).toBe(true);
        expect(state.error).toBeInstanceOf(ApiError);
        expect(state.error).toMatchObject({ status: 0, code: TIMEOUT });
        expect(seen.signal?.aborted).toBe(true);
    });

    it('a describe body that stalls for 30 seconds times out', async () => {
        vi.useFakeTimers();
        const body = stallingBody();
        const seen: { signal: AbortSignal | null } = { signal: null };
        vi.stubGlobal('fetch', async (_url: string, init?: RequestInit) => {
            seen.signal = init?.signal ?? null;
            return new Response(body.stream, { status: 200 });
        });

        // The headers arrive at once and the body stalls after its first byte.
        const state = observe(
            new PasteApi(ORIGIN).getDescriptor(ID, { describeToken: new Uint8Array(32) }),
        );
        await vi.advanceTimersByTimeAsync(29_999);
        expect(state.settled).toBe(false);
        await vi.advanceTimersByTimeAsync(1);
        expect(state.settled).toBe(true);
        expect(state.error).toMatchObject({ status: 0, code: TIMEOUT });
        expect(seen.signal?.aborted).toBe(true);
        expect(body.state.cancelled).toBe(true);
    });

    it('an error body that stalls for 30 seconds times out', async () => {
        vi.useFakeTimers();
        const body = stallingBody();
        vi.stubGlobal('fetch', async () => new Response(body.stream, { status: 500 }));

        // Retrieve has the long total deadline, so only the idle timer can end this.
        const state = observe(
            new PasteApi(ORIGIN).retrieve(ID, { describeToken: 'AA', revealToken: 'AA' }),
        );
        await vi.advanceTimersByTimeAsync(29_999);
        expect(state.settled).toBe(false);
        await vi.advanceTimersByTimeAsync(1);
        expect(state.settled).toBe(true);
        expect(state.error).toMatchObject({ status: 0, code: TIMEOUT });
    });
});
