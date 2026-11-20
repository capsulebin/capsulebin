/**
 * The bundled CLI on a real pseudo-terminal: the hidden password prompt and
 * the burn confirmation work, Ctrl-C and EOF stop the command with exit code 8
 * before the request the prompt guards, and the terminal is left with echo
 * and canonical mode on, whichever way the prompt ended. Escape sequences in
 * a text item reach the terminal only as visible escapes, unless `--raw`.
 */

import { randomBytes } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { startHarness, type Harness, type TerminalRun } from './pty';

const CTRL_C = '\x03';
const CTRL_D = '\x04';
const CONFIRM = 'Continue? [y/N] ';
const TIMEOUT = 30_000;

let harness: Harness;

/** Creates a capsule through the CLI and returns its share link. */
async function put(text: string, options: { password?: string; burn?: boolean } = {}) {
    const name = randomBytes(6).toString('hex');
    await writeFile(join(harness.dir, `${name}.txt`), text);
    const args = ['put', '--server', harness.url, '--json', `${name}.txt`];
    if (options.password !== undefined) {
        await writeFile(join(harness.dir, `${name}.pw`), options.password);
        args.push('--password-file', `${name}.pw`);
    }
    if (options.burn) args.push('--burn');
    const result = await harness.cli(args);
    expect(result.exitCode, result.stderr).toBe(0);
    return (JSON.parse(result.stdout) as { share_url: string }).share_url;
}

function retrieves(run: TerminalRun) {
    return run.requests.filter((request) => request.url.endsWith('/retrieve'));
}

function creates(run: TerminalRun) {
    return run.requests.filter(
        (request) =>
            request.method === 'POST' && new URL(request.url).pathname === '/api/v1/pastes',
    );
}

/** The `reason` of the `--json` error object on the terminal. */
function reason(run: TerminalRun): string | undefined {
    const line = /^\{"error":.*\}$/m.exec(run.text)?.[0];
    return line ? (JSON.parse(line) as { error: { reason: string } }).error.reason : undefined;
}

/** The terminal is back in its normal mode: echo and canonical input on. */
function expectRestored(run: TerminalRun) {
    const flags = run.stty.split(/[\s;]+/);
    expect(flags, run.stty).toContain('echo');
    expect(flags, run.stty).toContain('icanon');
    expect(flags).not.toContain('-echo');
    expect(flags).not.toContain('-icanon');
}

// No pseudo-terminals on Windows. Elsewhere a missing python3 fails the run.
describe.skipIf(process.platform === 'win32')('prompts on a terminal', () => {
    beforeAll(async () => {
        harness = await startHarness();
    }, TIMEOUT);

    afterAll(async () => {
        await harness?.close();
    });

    it(
        'asks for the password without echoing it and decrypts with it',
        async () => {
            const marker = `pw-${randomBytes(8).toString('hex')}`;
            const link = await put('protected plaintext\n', { password: marker });

            const run = await harness.terminal(
                ['get', link],
                [{ expect: 'Password: ', send: `${marker}\r` }],
            );

            expect(run.exitCode, run.text).toBe(0);
            expect(run.text).toContain('protected plaintext\n');
            expect(run.output.includes(marker)).toBe(false);
            expect(retrieves(run)).toHaveLength(1);
            expectRestored(run);
        },
        TIMEOUT,
    );

    it(
        'retrieves a burn capsule after the confirmation is answered y',
        async () => {
            const link = await put('burn plaintext\n', { burn: true });

            const run = await harness.terminal(['get', link], [{ expect: CONFIRM, send: 'y\r' }]);

            expect(run.exitCode, run.text).toBe(0);
            expect(run.text).toContain('burn plaintext\n');
            expect(retrieves(run)).toHaveLength(1);
            expectRestored(run);
        },
        TIMEOUT,
    );

    it.each([
        { answer: 'n\r', what: 'n', reason: 'declined' },
        { answer: CTRL_C, what: 'Ctrl-C', reason: 'interrupted' },
        { answer: CTRL_D, what: 'EOF', reason: 'input_closed' },
    ])(
        'stops before retrieving a burn capsule on $what at the confirmation',
        async ({ answer, reason: expected }) => {
            const link = await put('kept plaintext\n', { burn: true });

            const run = await harness.terminal(
                ['get', '--json', link],
                [{ expect: CONFIRM, send: answer }],
            );

            expect(run.exitCode, run.text).toBe(8);
            expect(reason(run)).toBe(expected);
            expect(retrieves(run)).toEqual([]);
            expect(run.text).not.toContain('kept plaintext');
            expectRestored(run);
        },
        TIMEOUT,
    );

    it.each([
        { answer: CTRL_C, what: 'Ctrl-C', reason: 'interrupted' },
        { answer: CTRL_D, what: 'EOF', reason: 'input_closed' },
    ])(
        'stops before retrieving on $what at the password prompt of get',
        async ({ answer, reason: expected }) => {
            const link = await put('protected plaintext\n', { password: 'correct horse' });

            const run = await harness.terminal(
                ['get', '--json', link],
                [{ expect: 'Password: ', send: answer }],
            );

            expect(run.exitCode, run.text).toBe(8);
            expect(reason(run)).toBe(expected);
            expect(retrieves(run)).toEqual([]);
            expectRestored(run);
        },
        TIMEOUT,
    );

    it(
        'stops before creating on Ctrl-C at the first password prompt of put',
        async () => {
            await writeFile(join(harness.dir, 'input.txt'), 'never uploaded\n');

            const run = await harness.terminal(
                ['put', '--server', harness.url, '--password', '--json', 'input.txt'],
                [{ expect: 'Password: ', send: CTRL_C }],
            );

            expect(run.exitCode, run.text).toBe(8);
            expect(reason(run)).toBe('interrupted');
            expect(creates(run)).toEqual([]);
            expectRestored(run);
        },
        TIMEOUT,
    );

    it(
        'creates a capsule with a password typed twice at the prompts of put',
        async () => {
            const marker = `pw-${randomBytes(8).toString('hex')}`;
            await writeFile(join(harness.dir, 'typed.txt'), 'typed password\n');

            const run = await harness.terminal(
                ['put', '--server', harness.url, '--password', '--json', 'typed.txt'],
                [
                    { expect: 'Password: ', send: `${marker}\r` },
                    { expect: 'Repeat password: ', send: `${marker}\r` },
                ],
            );

            expect(run.exitCode, run.text).toBe(0);
            expect(run.output.includes(marker)).toBe(false);
            expect(creates(run)).toHaveLength(1);
            expectRestored(run);
        },
        TIMEOUT,
    );

    it(
        'shows the escape sequences of a text item as literal escapes, raw only with --raw',
        async () => {
            const link = await put('title \x1b]0;MARK\x07 clear \x1b[2J end\n');

            const shown = await harness.terminal(['get', link], []);
            expect(shown.exitCode, shown.text).toBe(0);
            expect(shown.text).toContain('title \\x1b]0;MARK\\x07 clear \\x1b[2J end\n');
            expect(shown.output.includes(0x1b)).toBe(false);
            expect(shown.output.includes(0x07)).toBe(false);

            const raw = await harness.terminal(['get', '--raw', link], []);
            expect(raw.exitCode, raw.text).toBe(0);
            expect(raw.output.includes('\x1b]0;MARK\x07 clear \x1b[2J')).toBe(true);
        },
        TIMEOUT,
    );
});
