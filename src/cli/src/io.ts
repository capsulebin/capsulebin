/**
 * Everything the commands touch outside their own arguments, behind one
 * interface so tests can run a command without a terminal, a real stdin or the
 * user's environment.
 */

import { createInterface } from 'node:readline';

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
 */
function terminalPrompt(question: string, hidden: boolean): Promise<string> {
    return new Promise((resolve, reject) => {
        let muted = false;
        const output = {
            write: (chunk: string) => {
                if (!muted) process.stderr.write(chunk);
                return true;
            },
        };
        const rl = createInterface({
            input: process.stdin,
            output: output as unknown as NodeJS.WritableStream,
            terminal: true,
        });
        rl.on('SIGINT', () => {
            rl.close();
            process.stderr.write('\n');
            reject(new Error('interrupted'));
        });
        rl.question(question, (answer) => {
            rl.close();
            if (hidden) process.stderr.write('\n');
            resolve(answer);
        });
        muted = hidden;
    });
}
