/**
 * Test harness for the bundled CLI on a real (pseudo-)terminal.
 *
 * Bundles `src/main.ts` with the exact command of `npm run build` into a
 * temporary directory, serves the in-memory fake API from @capsulebin/core's
 * tests on a loopback HTTP socket, and runs the bundle in a pseudo-terminal
 * opened by python3's standard `pty` module (`pty-driver.py`), so prompts see
 * a terminal exactly as on a user's machine.
 */

import { spawn, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';
import { createFakeServer, type FakeServer } from '../../core/tests/fake-server';

const CLI_DIR = resolve(import.meta.dirname, '..');
const REPO_ROOT = resolve(CLI_DIR, '../..');
const DRIVER = join(import.meta.dirname, 'pty-driver.py');

/** One step of a dialogue: wait until `expect` is on the terminal, then type `send`. */
export interface Step {
    expect: string;
    send: string;
}

export interface TerminalRun {
    /** Exit code of the CLI; negative when it was ended by that signal. */
    exitCode: number;
    /** Everything the terminal showed (stdout and stderr), as raw bytes. */
    output: Buffer;
    /** `output` as UTF-8 with the terminal's `\r\n` turned into `\n`. */
    text: string;
    /** `stty -a` on the same terminal, right after the CLI exited. */
    stty: string;
    /** The fake API's requests made during this run. */
    requests: FakeServer['requests'];
}

export interface Harness {
    /** Base URL of the fake API (loopback HTTP), for `--server`. */
    url: string;
    api: FakeServer;
    /** A scratch directory, the CLI's working directory. */
    dir: string;
    /** Runs the CLI without a terminal (stdin closed), e.g. to create a capsule. */
    cli(args: string[]): Promise<{ exitCode: number; stdout: string; stderr: string }>;
    /** Runs the CLI in a pseudo-terminal and plays `steps`. */
    terminal(args: string[], steps: Step[], options?: { timeoutMs?: number }): Promise<TerminalRun>;
    close(): Promise<void>;
}

/** Fails (rather than skips) when python3 is missing: the terminal tests would not run. */
export function requirePython(): void {
    const probe = spawnSync('python3', ['-I', '-c', 'import pty, termios'], { encoding: 'utf8' });
    if (probe.error || probe.status !== 0) {
        throw new Error(
            'the terminal tests need python3 (with its pty and termios modules) on PATH: ' +
                (probe.error?.message ?? probe.stderr.trim()),
        );
    }
}

/**
 * `npm run build -w @capsulebin/cli`, writing to `outfile` instead of
 * `dist/cli.js`: the script itself is run, so the options cannot drift.
 */
function bundleCli(outfile: string): void {
    const manifest = JSON.parse(readFileSync(join(CLI_DIR, 'package.json'), 'utf8')) as {
        scripts: { build: string };
    };
    const build = manifest.scripts.build;
    if (!build.includes('--outfile=dist/cli.js') || outfile.includes("'")) {
        throw new Error(`cannot redirect the CLI build to ${outfile}: ${build}`);
    }
    const command = build.replaceAll('dist/cli.js', `'${outfile}'`);
    const bins = [join(CLI_DIR, 'node_modules/.bin'), join(REPO_ROOT, 'node_modules/.bin')];
    const result = spawnSync('sh', ['-c', command], {
        cwd: CLI_DIR,
        encoding: 'utf8',
        env: { ...process.env, PATH: [...bins, process.env['PATH'] ?? ''].join(delimiter) },
    });
    if (result.error || result.status !== 0) {
        throw new Error(`bundling the CLI failed: ${result.error?.message ?? result.stderr}`);
    }
}

/** Serves `api` (a `fetch` stand-in) over HTTP. */
async function forward(api: FakeServer, req: IncomingMessage, res: ServerResponse) {
    const chunks: Buffer[] = [];
    for await (const chunk of req) chunks.push(chunk as Buffer);
    const body = Buffer.concat(chunks).toString('utf8');
    const headers = new Headers();
    for (const [name, value] of Object.entries(req.headers)) {
        if (typeof value === 'string') headers.set(name, value);
    }
    const response = await api.fetch(`http://${req.headers.host}${req.url}`, {
        method: req.method ?? 'GET',
        headers,
        ...(body ? { body } : {}),
    });
    res.writeHead(response.status, Object.fromEntries(response.headers));
    res.end(Buffer.from(await response.arrayBuffer()));
}

function collect(child: ReturnType<typeof spawn>, input?: string) {
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    child.stdout?.on('data', (chunk: Buffer) => stdout.push(chunk));
    child.stderr?.on('data', (chunk: Buffer) => stderr.push(chunk));
    child.stdin?.end(input);
    return new Promise<{ exitCode: number; stdout: string; stderr: string }>((done, fail) => {
        child.on('error', fail);
        child.on('close', (code) =>
            done({
                exitCode: code ?? -1,
                stdout: Buffer.concat(stdout).toString('utf8'),
                stderr: Buffer.concat(stderr).toString('utf8'),
            }),
        );
    });
}

export async function startHarness(): Promise<Harness> {
    requirePython();
    const dir = await mkdtemp(join(tmpdir(), 'capsulebin-terminal-'));
    const bundle = join(dir, 'cli.js');
    bundleCli(bundle);

    // Requests reach it with absolute URLs, so its default origin is never used.
    const api = createFakeServer();
    const http = createServer((req, res) => {
        forward(api, req, res).catch((err: unknown) => {
            res.writeHead(500).end(String(err));
        });
    });
    await new Promise<void>((done) => http.listen(0, '127.0.0.1', done));
    const url = `http://127.0.0.1:${(http.address() as AddressInfo).port}`;

    // Only what the CLI needs: no CAPSULEBIN_* variables from the developer's shell.
    const env = { PATH: process.env['PATH'] ?? '', HOME: dir, TERM: 'xterm' };
    const argv = (args: string[]) => [process.execPath, bundle, ...args];

    return {
        url,
        api,
        dir,
        cli: (args) => {
            const [command, ...rest] = argv(args);
            return collect(spawn(command!, rest, { cwd: dir, env }));
        },
        terminal: async (args, steps, options = {}) => {
            const timeoutMs = options.timeoutMs ?? 15_000;
            const before = api.requests.length;
            const spec = { argv: argv(args), env, cwd: dir, timeout: timeoutMs / 1000, steps };
            const driver = await collect(
                spawn('python3', ['-I', DRIVER], { cwd: dir }),
                JSON.stringify(spec),
            );
            if (driver.exitCode !== 0) {
                throw new Error(`pty-driver.py failed (${driver.exitCode}): ${driver.stderr}`);
            }
            const result = JSON.parse(driver.stdout) as {
                exit?: number;
                stty?: string;
                error?: string;
                output: string;
            };
            const output = Buffer.from(result.output, 'base64');
            const text = output.toString('utf8').replaceAll('\r\n', '\n');
            if (result.error !== undefined) {
                throw new Error(`${result.error}; the terminal showed:\n${text}`);
            }
            return {
                exitCode: result.exit!,
                output,
                text,
                stty: result.stty!,
                requests: api.requests.slice(before),
            };
        },
        close: async () => {
            await new Promise((done) => http.close(done));
            await rm(dir, { recursive: true, force: true });
        },
    };
}
