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
     * returns `limit + 1` of them and reads no further). Without a limit, all
     * of it. Only called when stdin is the content source or carries a link.
     */
    readStdin(limit?: number): Promise<Uint8Array>;
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
    limit = Infinity,
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
