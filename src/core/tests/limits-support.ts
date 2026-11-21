/**
 * Helpers shared by the API client's read-bound tests (`limits.test.ts`,
 * `limits.acceptance.test.ts`): a sealed one-item paste, a way to inspect a
 * promise without awaiting it, and response bodies that never end or stall.
 */

import { Paste } from '../src/paste';

/** The chunk size of `endlessBody`. */
export const CHUNK = 64 * 1024;

export async function sealedPaste() {
    const paste = new Paste({ expiresIn: 60, burnAfterRetrieval: false });
    paste.addItem({
        id: '01J00000000000000000000000',
        name: 'x.txt',
        size: 1,
        mediaType: 'text/plain',
        presentation: 'plain',
        bytes: new Uint8Array([0x78]),
    });
    return paste.seal();
}

/** Settles `promise` into a value that can be inspected without awaiting it. */
export function observe(promise: Promise<unknown>): { settled: boolean; error: unknown } {
    const state = { settled: false, error: undefined as unknown };
    promise.then(
        () => {
            state.settled = true;
        },
        (err: unknown) => {
            state.settled = true;
            state.error = err;
        },
    );
    return state;
}

/** A body of `total` bytes that is produced only on demand, counting what was read. */
export function endlessBody(total: number) {
    const state = { read: 0, cancelled: false };
    const stream = new ReadableStream<Uint8Array>(
        {
            pull(controller) {
                if (state.read >= total) {
                    controller.close();
                    return;
                }
                const chunk = new Uint8Array(Math.min(CHUNK, total - state.read)).fill(0x20);
                state.read += chunk.length;
                controller.enqueue(chunk);
            },
            cancel() {
                state.cancelled = true;
            },
        },
        { highWaterMark: 0 },
    );
    return { stream, state };
}

/** A body that delivers one byte and then never settles its next read. */
export function stallingBody() {
    const state = { cancelled: false };
    let sent = false;
    const stream = new ReadableStream<Uint8Array>(
        {
            pull(controller) {
                if (!sent) {
                    sent = true;
                    controller.enqueue(new Uint8Array([0x7b]));
                }
                return new Promise(() => undefined);
            },
            cancel() {
                state.cancelled = true;
            },
        },
        { highWaterMark: 0 },
    );
    return { stream, state };
}
