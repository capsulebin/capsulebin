/**
 * Everything the commands touch outside their own arguments, behind one
 * interface so tests can run a command without a terminal, a real stdin or the
 * user's environment.
 */

import { createInterface } from 'node:readline';
import { Writable } from 'node:stream';
import { promptStopped } from './exit';

export interface Io {
    stdout: { write(chunk: string | Uint8Array): unknown };
    stderr: { write(chunk: string): unknown };
    /**
     * Reads stdin to its end, or stops after more than `limit` bytes (then
     * returns `limit + 1` of them and reads no further). Only called when
     * stdin is the content source.
     */
    readStdin(limit: number): Promise<Uint8Array>;
    /**
     * The first line of stdin (see {@link readFirstLine}), or `null` when it
     * is longer than `limit` bytes. Does not wait for the end of stdin. Only
     * called when stdin carries a link.
     */
    readStdinLine(limit: number): Promise<Uint8Array | null>;
    /** Whether an interactive prompt is possible (stdin and stderr are a terminal). */
    interactive: boolean;
    /** Asks a question on the terminal; `hidden` suppresses the echo (passwords). */
    prompt(question: string, hidden: boolean): Promise<string>;
    env: Record<string, string | undefined>;
    cwd: string;
    /** Waits between retries. Replaced by a no-op in tests. */
    sleep(ms: number): Promise<void>;
}

export function processIo(): Io {
    return {
        stdout: process.stdout,
        stderr: process.stderr,
        readStdin: (limit) => readBounded(process.stdin, limit),
        readStdinLine: (limit) => readFirstLine(process.stdin, limit),
        interactive: Boolean(process.stdin.isTTY && process.stderr.isTTY),
        prompt: terminalPrompt,
        env: process.env,
        cwd: process.cwd(),
        sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    };
}

/**
 * Collects `chunks` until they end or exceed `limit` bytes. Over the limit the
 * iteration is left early, which destroys a Node stream, and the result is cut
 * to `limit + 1` bytes: enough for the caller to see the overflow, never more
 * than the limit plus one chunk read.
 */
export async function readBounded(
    chunks: AsyncIterable<Uint8Array> | Iterable<Uint8Array>,
    limit: number,
): Promise<Uint8Array> {
    const parts: Uint8Array[] = [];
    let total = 0;
    for await (const chunk of chunks) {
        parts.push(chunk);
        total += chunk.length;
        if (total > limit) break;
    }
    const bytes = new Uint8Array(Buffer.concat(parts));
    return total > limit ? bytes.subarray(0, limit + 1) : bytes;
}

/**
 * The first line of `chunks` without its line ending: `\n`, a preceding `\r`
 * stripped; the end of the input ends the line as well. `null` when the line
 * is longer than `limit` bytes. The iteration is left at the line ending, or
 * as soon as the line is too long, which destroys a Node stream: a source
 * that stays open does not block, and never more than the limit plus one
 * chunk is read.
 */
export async function readFirstLine(
    chunks: AsyncIterable<Uint8Array> | Iterable<Uint8Array>,
    limit: number,
): Promise<Uint8Array | null> {
    const parts: Uint8Array[] = [];
    let total = 0;
    let terminated = false;
    for await (const chunk of chunks) {
        const newline = chunk.indexOf(0x0a);
        terminated = newline !== -1;
        const part = terminated ? chunk.subarray(0, newline) : chunk;
        parts.push(part);
        total += part.length;
        // One byte above the limit may still be the `\r` of a line ending.
        if (terminated || total > limit + 1) break;
    }
    const bytes = new Uint8Array(Buffer.concat(parts));
    const line = terminated && bytes.at(-1) === 0x0d ? bytes.subarray(0, -1) : bytes;
    return line.length > limit ? null : line;
}

/**
 * Prompts on stderr and reads one line from the terminal. With `hidden`, the
 * typed characters are not echoed — the readline output is muted after the
 * question has been written.
 *
 * readline puts the terminal into raw mode for the question and restores it
 * when the interface closes, which happens on every way out: an answer,
 * Ctrl-C and EOF (Ctrl-D on an empty line). The last two reject with exit
 * code 8, so the command stops before the request the prompt guards.
 */
function terminalPrompt(question: string, hidden: boolean): Promise<string> {
    return new Promise((resolve, reject) => {
        let muted = false;
        let settled = false;
        const output = new Writable({
            write(chunk: Buffer | string, _encoding, callback) {
                if (!muted) process.stderr.write(chunk);
                callback();
            },
        });
        const rl = createInterface({ input: process.stdin, output, terminal: true });
        const stop = (reason: 'interrupted' | 'input_closed') => {
            settled = true;
            rl.close();
            process.stderr.write('\n');
            reject(promptStopped(reason));
        };
        rl.on('SIGINT', () => stop('interrupted'));
        rl.on('close', () => {
            if (!settled) stop('input_closed');
        });
        rl.question(question, (answer) => {
            settled = true;
            rl.close();
            if (hidden) process.stderr.write('\n');
            resolve(answer);
        });
        muted = hidden;
    });
}
