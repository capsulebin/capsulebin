/**
 * Everything the commands touch outside their own arguments, behind one
 * interface so tests can run a command without a terminal, a real stdin or the
 * user's environment.
 */

import { createInterface } from 'node:readline';
import { Writable } from 'node:stream';
import { promptStopped } from './exit';

export interface Io {
    stdout: { write(chunk: string | Uint8Array): unknown; isTTY?: boolean };
    stderr: { write(chunk: string): unknown };
    /** Reads all of stdin. Only called when stdin is the content source. */
    readStdin(): Promise<Uint8Array>;
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
        readStdin: async () => {
            const chunks: Buffer[] = [];
            for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
            return new Uint8Array(Buffer.concat(chunks));
        },
        interactive: Boolean(process.stdin.isTTY && process.stderr.isTTY),
        prompt: terminalPrompt,
        env: process.env,
        cwd: process.cwd(),
        sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    };
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
