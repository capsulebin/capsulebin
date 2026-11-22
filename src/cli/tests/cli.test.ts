/**
 * The CLI end to end against the in-memory API from @capsulebin/core's tests:
 * argument handling, put → get → delete round trips, the exit codes scripts
 * rely on, and the rules that protect secrets and burn capsules.
 */

import {
    access,
    chmod,
    lstat,
    mkdir,
    mkdtemp,
    readdir,
    readFile,
    readlink,
    stat,
    symlink,
    truncate,
    writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    decodeUtf8,
    encodeUtf8,
    fromBase64url,
    generateUlid,
    MAX_CAPSULE_BYTES,
    Paste,
    PasteApi,
    toBase64url,
} from '@capsulebin/core';
import { run, parseDuration } from '../src/cli';
import { safeFileName } from '../src/commands/get';
import { type Io, readBounded, readFirstLine } from '../src/io';
import { buildItem } from '../src/items';
import { escapeDiagnostic } from '../src/terminal';
import { createFakeServer, type FakeServer } from '../../core/tests/fake-server';

// While set, every file write fails after the file was opened;
// `failOpenNumber` makes only the write through the n-th opened file fail.
// `get` creates every file in place: `link` and `rename` must not be used.
const fsHooks = vi.hoisted(() => ({
    failWrites: false,
    failOpenNumber: undefined as number | undefined,
    opens: 0,
    /** Read streams opened (by `put` for its input files). */
    readStreams: 0,
}));
vi.mock('node:fs', async (importOriginal) => {
    const fs = await importOriginal<typeof import('node:fs')>();
    return {
        ...fs,
        createReadStream: (...args: Parameters<typeof fs.createReadStream>) => {
            fsHooks.readStreams++;
            return fs.createReadStream(...args);
        },
    };
});
vi.mock('node:fs/promises', async (importOriginal) => {
    const fs = await importOriginal<typeof import('node:fs/promises')>();
    return {
        ...fs,
        open: async (...args: Parameters<typeof fs.open>) => {
            const handle = await fs.open(...args);
            fsHooks.opens++;
            if (fsHooks.failWrites || fsHooks.opens === fsHooks.failOpenNumber) {
                handle.writeFile = async () => {
                    throw Object.assign(new Error('no space left on device'), { code: 'ENOSPC' });
                };
            }
            return handle;
        },
        link: async () => {
            throw new Error('link must not be used');
        },
        rename: async () => {
            throw new Error('rename must not be used');
        },
    };
});

const SERVER = 'https://capsule.test';
/** A management (delete) link on {@link SERVER}, unanchored. */
const MANAGE_LINK = new RegExp(`${SERVER}/manage/[A-Za-z0-9_-]{22}#v1\\.[A-Za-z0-9_-]+`);

let server: FakeServer;
let cwd: string;

beforeEach(async () => {
    server = createFakeServer(SERVER);
    vi.stubGlobal('fetch', server.fetch);
    cwd = await mkdtemp(join(tmpdir(), 'capsulebin-cli-'));
});

afterEach(() => {
    vi.unstubAllGlobals();
    fsHooks.failWrites = false;
    fsHooks.failOpenNumber = undefined;
    fsHooks.opens = 0;
    fsHooks.readStreams = 0;
    vi.useRealTimers();
});

interface TestIo extends Io {
    out(): string;
    err(): string;
    prompts: string[];
}

function testIo(
    options: {
        stdin?: string | Uint8Array | AsyncIterable<Uint8Array>;
        env?: Record<string, string>;
        interactive?: boolean;
        answers?: string[];
    } = {},
): TestIo {
    const out: Uint8Array[] = [];
    const err: string[] = [];
    const answers = [...(options.answers ?? [])];
    const prompts: string[] = [];
    const encoder = new TextEncoder();
    const stdinChunks = () => {
        const stdin = options.stdin ?? new Uint8Array();
        if (typeof stdin === 'string') return [encoder.encode(stdin)];
        return stdin instanceof Uint8Array ? [stdin] : stdin;
    };
    return {
        stdout: {
            write: (chunk) => out.push(typeof chunk === 'string' ? encoder.encode(chunk) : chunk),
        },
        stderr: { write: (chunk) => err.push(chunk) },
        readStdin: (limit) => readBounded(stdinChunks(), limit),
        readStdinLine: (limit) => readFirstLine(stdinChunks(), limit),
        interactive: options.interactive ?? false,
        prompt: async (question) => {
            prompts.push(question);
            const answer = answers.shift();
            if (answer === undefined) throw new Error('unexpected prompt: ' + question);
            return answer;
        },
        env: options.env ?? {},
        cwd,
        sleep: async () => {},
        out: () => Buffer.concat(out).toString('utf8'),
        err: () => err.join(''),
        prompts,
    };
}

/** The paths a `get` reported on stderr (`wrote <path>`). */
function wrotePaths(io: TestIo): string[] {
    return [...io.err().matchAll(/^wrote (.*)$/gm)].map((match) => match[1]!);
}

/** The content of the one file a `get` wrote; its stdout stayed empty. */
async function written(io: TestIo): Promise<string> {
    expect(io.out()).toBe('');
    const paths = wrotePaths(io);
    expect(paths, io.err()).toHaveLength(1);
    return readFile(paths[0]!, 'utf8');
}

async function putText(text: string, ...args: string[]) {
    const io = testIo({ stdin: text });
    const code = await run(['put', '--server', SERVER, '--json', ...args], io);
    expect(code, io.err()).toBe(0);
    return JSON.parse(io.out()) as {
        id: string;
        share_url: string;
        delete_url: string;
        expires_at: number | null;
        burn_after_retrieval: boolean;
    };
}

describe('put → get → delete', () => {
    it('round-trips stdin through a capsule and back to a file', async () => {
        const created = await putText('hello from stdin\n');
        expect(created.share_url).toMatch(new RegExp(`^${SERVER}/p/[A-Za-z0-9_-]{22}#v1\\.`));
        expect(created.delete_url).toMatch(new RegExp(`^${SERVER}/manage/${created.id}#v1\\.`));

        const io = testIo();
        expect(await run(['get', created.share_url], io)).toBe(0);
        expect(await written(io)).toBe('hello from stdin\n');
        const [path] = wrotePaths(io);
        const dir = dirname(path!);
        expect(dirname(dir)).toBe(cwd);
        expect(basename(dir)).toMatch(
            new RegExp(`^capsule-\\d{4}-\\d{2}-\\d{2}-${created.id.slice(0, 8)}$`),
        );
        expect(basename(path!)).toBe('stdin');
    });

    it('deletes with the management link, after which get reports not available', async () => {
        const created = await putText('to be deleted');
        expect(await run(['delete', created.delete_url], testIo())).toBe(0);

        expect(await run(['get', created.share_url], testIo())).toBe(3);
    });

    it('accepts links with a language prefix, as copied from the address bar', async () => {
        const created = await putText('hallo');
        const withPrefix = (url: string) => url.replace(`${SERVER}/`, `${SERVER}/de/`);

        const io = testIo();
        expect(await run(['get', withPrefix(created.share_url)], io), io.err()).toBe(0);
        expect(await written(io)).toBe('hallo');

        expect(await run(['delete', withPrefix(created.delete_url)], testIo())).toBe(0);
        expect(server.pastes.has(created.id)).toBe(false);
    });

    it('packs several files into one capsule and writes them back to a directory', async () => {
        await writeFile(join(cwd, 'notes.md'), '# Notes\n');
        await writeFile(join(cwd, 'main.rs'), 'fn main() {}\n');
        await writeFile(join(cwd, 'blob.bin'), new Uint8Array([0, 1, 2, 255]));

        const io = testIo();
        expect(
            await run(['put', '-s', SERVER, 'notes.md', 'main.rs', 'blob.bin'], io),
            io.err(),
        ).toBe(0);
        const shareUrl = io.out().trim();
        expect(io.err()).toContain('/manage/');

        const getIo = testIo();
        expect(await run(['get', shareUrl, '-o', 'out', '--json'], getIo)).toBe(0);
        const result = JSON.parse(getIo.out());
        expect(
            result.items.map((i: { name: string; presentation: string; language: string }) => [
                i.name,
                i.presentation,
                i.language,
            ]),
        ).toEqual([
            ['notes.md', 'markdown', null],
            ['main.rs', 'code', 'rust'],
            ['blob.bin', 'download', null],
        ]);
        expect(await readFile(join(cwd, 'out', 'blob.bin'))).toEqual(Buffer.from([0, 1, 2, 255]));
        expect(await readFile(join(cwd, 'out', 'main.rs'), 'utf8')).toBe('fn main() {}\n');
    });
});

describe('link channels', () => {
    const withPrefix = (url: string) => url.replace(`${SERVER}/`, `${SERVER}/de/`);

    it.each([
        ['plain', (url: string) => url],
        ['with a /de/ segment', withPrefix],
    ])('round-trips with the links on stdin (%s)', async (_, form) => {
        const created = await putText('from stdin');

        const io = testIo({ stdin: form(created.share_url) + '\n' });
        expect(await run(['get', '-'], io), io.err()).toBe(0);
        expect(await written(io)).toBe('from stdin');

        const del = testIo({ stdin: form(created.delete_url) + '\r\nignored\n' });
        expect(await run(['delete', '-'], del), del.err()).toBe(0);
        expect(server.pastes.has(created.id)).toBe(false);
    });

    it.each([
        ['plain', (url: string) => url],
        ['with a /de/ segment', withPrefix],
    ])('round-trips with the links in files (%s)', async (_, form) => {
        const created = await putText('from a file');
        await writeFile(join(cwd, 'share'), form(created.share_url) + '\r\n');
        await writeFile(join(cwd, 'manage'), form(created.delete_url));

        const io = testIo();
        expect(await run(['get', '--link-file', 'share'], io), io.err()).toBe(0);
        expect(await written(io)).toBe('from a file');

        const del = testIo();
        expect(await run(['delete', '--link-file', join(cwd, 'manage')], del), del.err()).toBe(0);
        expect(server.pastes.has(created.id)).toBe(false);
    });

    it('refuses an ambiguous, missing, empty or unreadable link without quoting it', async () => {
        const created = await putText('x');
        await writeFile(join(cwd, 'share'), created.share_url + '\n');
        await writeFile(join(cwd, 'empty'), '');
        await writeFile(join(cwd, 'blank-line'), '\n' + created.share_url + '\n');
        const before = server.requests.length;

        for (const command of ['get', 'delete']) {
            const cases: [string[], string?][] = [
                [[command, created.share_url, '--link-file', 'share']],
                [[command, '-', '--link-file', 'share'], created.share_url],
                [[command, created.share_url, created.share_url]],
                [[command]],
                [[command, '-']],
                [[command, '-'], '\n' + created.share_url],
                [[command, '--link-file', 'empty']],
                [[command, '--link-file', 'blank-line']],
                [[command, '--link-file', 'missing']],
                [[command, '--link-file', cwd]],
            ];
            for (const [argv, stdin] of cases) {
                const io = testIo(stdin === undefined ? {} : { stdin });
                expect(await run(argv, io), argv.join(' ')).toBe(2);
                expect(io.err()).not.toContain('#v1.');
                expect(io.err()).not.toContain(created.id);
            }
        }
        expect(server.requests).toHaveLength(before);
    });
});

/** Lets the fake server handle every request, then loses its response. */
function loseResponses() {
    vi.stubGlobal('fetch', async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
        await server.fetch(input, init);
        throw new TypeError('connection reset');
    });
}

describe('put --delete-link-file', () => {
    it('writes the management link to a new owner-only file instead of stderr', async () => {
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'private' });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io), io.err()).toBe(0);
        const deleteUrl = await readFile(path, 'utf8');
        expect(deleteUrl).toMatch(new RegExp(`^${MANAGE_LINK.source}\\n$`));
        if (process.platform !== 'win32') expect((await stat(path)).mode & 0o777).toBe(0o600);
        expect(io.err()).not.toContain('#v1.');
        expect(io.err()).toContain(path);
        expect(io.out()).toMatch(/\/p\//);

        expect(await run(['delete', '--link-file', path], testIo())).toBe(0);
        expect(server.pastes.size).toBe(0);
    });

    it('names the absolute path in JSON and leaves out delete_url', async () => {
        const io = testIo({ stdin: 'private' });
        expect(
            await run(['put', '-s', SERVER, '--json', '--delete-link-file', 'link'], io),
            io.err(),
        ).toBe(0);
        const result = JSON.parse(io.out());
        expect(result.delete_link_file).toBe(join(cwd, 'link'));
        expect(result).not.toHaveProperty('delete_url');
        expect(io.out()).not.toContain('/manage/');
        expect(io.err()).not.toContain('#v1.');
    });

    it('refuses an existing file or symlink before any request', async () => {
        await writeFile(join(cwd, 'taken'), 'keep me');
        await symlink(join(cwd, 'nowhere'), join(cwd, 'dangling'));
        for (const name of ['taken', 'dangling']) {
            const io = testIo({ stdin: 'x' });
            expect(await run(['put', '-s', SERVER, '--delete-link-file', name], io)).toBe(2);
            expect(io.err()).toContain(join(cwd, name));
        }
        expect(server.requests).toHaveLength(0);
        expect(await readFile(join(cwd, 'taken'), 'utf8')).toBe('keep me');
        expect((await lstat(join(cwd, 'dangling'))).isSymbolicLink()).toBe(true);
        await expect(access(join(cwd, 'nowhere'))).rejects.toThrow();
    });

    it('keeps the file when the capsule was stored but every response was lost', async () => {
        const shareUrl = vi.spyOn(Paste.prototype, 'shareUrl');
        loseResponses();
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x' });
        expect(
            await run(['put', '-s', SERVER, '--retries', '2', '--delete-link-file', path], io),
        ).toBe(7);
        expect(server.requests).toHaveLength(3);
        expect(server.pastes.size).toBe(1);
        expect(await readFile(path, 'utf8')).toMatch(new RegExp(`^${MANAGE_LINK.source}\\n$`));
        if (process.platform !== 'win32') expect((await stat(path)).mode & 0o777).toBe(0o600);
        expect(io.err()).toContain('the capsule may exist');
        expect(io.err()).toContain(path);
        expect(io.err()).not.toContain('#v1.');
        expect(io.out()).toBe('');

        vi.stubGlobal('fetch', server.fetch);
        const link = shareUrl.mock.results[0]!.value as string;
        shareUrl.mockRestore();
        expect(await run(['delete', '--link-file', path], testIo())).toBe(0);
        expect(await run(['get', link], testIo())).toBe(3);
    });

    it('keeps the file after a rejected create (401)', async () => {
        server.creationToken = 'cbt_secret';
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x', env: { CAPSULEBIN_TOKEN: 'cbt_wrong' } });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io)).toBe(6);
        expect(server.requests).toHaveLength(1);
        expect(await readFile(path, 'utf8')).toMatch(MANAGE_LINK);
        expect(io.err()).toContain('the capsule may exist');
        expect(io.err()).toContain(path);
        expect(io.err()).not.toContain('#v1.');
    });

    it('keeps the file after a refused redirect', async () => {
        let calls = 0;
        vi.stubGlobal('fetch', async () => {
            calls++;
            return new Response(null, { status: 307, headers: { Location: 'https://x.test/' } });
        });
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io)).toBe(7);
        expect(calls).toBe(1);
        expect(await readFile(path, 'utf8')).toMatch(MANAGE_LINK);
        expect(io.err()).toContain('the capsule may exist');
        expect(io.err()).toContain(path);
        expect(io.err()).not.toContain('#v1.');
    });

    it('removes the file and sends nothing when the link cannot be written', async () => {
        fsHooks.failWrites = true;
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--json', '--delete-link-file', path], io)).toBe(1);
        expect(server.requests).toHaveLength(0);
        await expect(access(path)).rejects.toThrow();
        expect(io.err()).not.toContain('may exist');
        expect(JSON.parse(io.out()).error).not.toHaveProperty('delete_link_file');
    });

    it('names the file in the JSON error and shows the link nowhere', async () => {
        loseResponses();
        const io = testIo({ stdin: 'x' });
        expect(
            await run(
                ['put', '-s', SERVER, '--json', '--retries', '0', '--delete-link-file', 'link'],
                io,
            ),
        ).toBe(7);
        expect(JSON.parse(io.out())).toEqual({
            error: {
                reason: 'network_error',
                message: 'could not reach the server',
                delete_link_file: join(cwd, 'link'),
            },
        });
        expect(io.err()).toContain(join(cwd, 'link'));
        for (const output of [io.out(), io.err()]) {
            expect(output).not.toContain('#v1.');
            expect(output).not.toContain('/manage/');
        }
    });
});

describe('put without --delete-link-file, after a failed create', () => {
    it('prints the delete link of a capsule that may exist on stderr', async () => {
        loseResponses();
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--retries', '1'], io)).toBe(7);
        expect(server.pastes.size).toBe(1);
        expect(io.out()).toBe('');
        const line = io.err().trimEnd().split('\n').at(-1)!;
        expect(line).toContain('the capsule may exist');
        const link = /https:\/\/capsule\.test\/manage\/[A-Za-z0-9_-]{22}#v1\.[A-Za-z0-9_-]+$/.exec(
            line,
        );
        expect(link, io.err()).not.toBeNull();

        vi.stubGlobal('fetch', server.fetch);
        expect(await run(['delete', link![0]], testIo())).toBe(0);
        expect(server.pastes.size).toBe(0);
    });

    it('prints it after a rejected create and puts delete_url into the JSON error', async () => {
        server.creationToken = 'cbt_secret';
        const text = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER], text)).toBe(6);
        expect(text.err()).toContain('the capsule may exist');
        expect(text.err()).toMatch(/\/manage\/[A-Za-z0-9_-]{22}#v1\./);

        const json = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--json'], json)).toBe(6);
        const { error } = JSON.parse(json.out());
        expect(Object.keys(error)).toEqual(['reason', 'message', 'delete_url']);
        expect(error.reason).toBe('unauthorized');
        expect(error.delete_url).toMatch(new RegExp(`^${MANAGE_LINK.source}$`));
        expect(json.err()).toContain(error.delete_url);
    });

    it.each([
        ['a usage error', ['--retries', 'many']],
        ['an unreadable input', ['missing.txt']],
        ['an empty password file', ['--password-file', 'empty']],
    ])('prints no delete link after %s, before any request', async (_, args) => {
        await writeFile(join(cwd, 'empty'), '');
        for (const extra of [[], ['--json'], ['--json', '--delete-link-file', 'link']]) {
            const io = testIo({ stdin: 'x' });
            expect(await run(['put', '-s', SERVER, ...args, ...extra], io)).toBe(2);
            expect(io.err()).not.toContain('may exist');
            expect(io.err()).not.toContain('/manage/');
            expect(io.out()).not.toContain('/manage/');
            if (extra.length > 0) {
                expect(Object.keys(JSON.parse(io.out()).error)).toEqual(['reason', 'message']);
            }
        }
        expect(server.requests).toHaveLength(0);
        await expect(access(join(cwd, 'link'))).rejects.toThrow();
    });
});

describe('put options', () => {
    it('omits expires_in unless --expires is given, so the server default applies', async () => {
        await putText('x');
        expect(JSON.parse(server.requests[0]!.body)).not.toHaveProperty('expires_in');

        await putText('y', '--expires', '12h');
        expect(JSON.parse(server.requests[1]!.body).expires_in).toBe(43_200);

        await putText('z', '--expires', 'never');
        expect(JSON.parse(server.requests[2]!.body).expires_in).toBeNull();
    });

    it('sends the creation token from CAPSULEBIN_TOKEN or --token-file, never on reads', async () => {
        server.creationToken = 'cbt_from_env';
        const io = testIo({ stdin: 'x', env: { CAPSULEBIN_TOKEN: 'cbt_from_env' } });
        expect(await run(['put', '-s', SERVER], io)).toBe(0);

        server.creationToken = 'cbt_from_file';
        await writeFile(join(cwd, 'token'), 'cbt_from_file\n');
        const fileIo = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--token-file', 'token'], fileIo)).toBe(0);

        const getIo = testIo();
        await run(['get', fileIo.out().trim()], getIo);
        const reads = server.requests.filter(
            (r) => r.method !== 'POST' || r.url.endsWith('/retrieve'),
        );
        expect(reads.length).toBeGreaterThan(0);
        for (const r of reads) expect(r.headers.get('Authorization')).toBeNull();
    });

    it('explains a missing token with exit code 6', async () => {
        server.creationToken = 'cbt_secret';
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER], io)).toBe(6);
        expect(io.err()).toContain('creation token');
    });

    it('retries a transient failure with the same sealed request', async () => {
        let calls = 0;
        const realFetch = server.fetch;
        vi.stubGlobal('fetch', async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
            calls++;
            const response = await realFetch(input, init);
            // The first attempt lands but its response is lost.
            if (calls === 1) throw new TypeError('socket hang up');
            return response;
        });

        const io = testIo({ stdin: 'retry me' });
        expect(await run(['put', '-s', SERVER], io), io.err()).toBe(0);
        expect(calls).toBe(2);
        expect(server.pastes.size).toBe(1);
        expect(server.requests[0]!.body).toBe(server.requests[1]!.body);
        expect(io.err()).toContain('retrying');
    });

    it('gives up after --retries and reports the server as unreachable', async () => {
        vi.stubGlobal('fetch', async () => new Response('{}', { status: 503 }));
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--retries', '2'], io)).toBe(7);
        expect(io.err().match(/retrying/g)).toHaveLength(2);
    });

    it('protects with a password from CAPSULEBIN_PASSWORD and needs it to open', async () => {
        const io = testIo({ stdin: 'secret note', env: { CAPSULEBIN_PASSWORD: 'hunter22' } });
        expect(await run(['put', '-s', SERVER, '--password'], io)).toBe(0);
        const shareUrl = io.out().trim();

        // Neither request contains the password.
        for (const r of server.requests) expect(r.body).not.toContain('hunter22');

        expect(await run(['get', shareUrl], testIo())).toBe(8);
        expect(
            await run(
                ['get', '-s', SERVER, shareUrl],
                testIo({ env: { CAPSULEBIN_PASSWORD: 'wrong' } }),
            ),
        ).toBe(3);
        const ok = testIo({ env: { CAPSULEBIN_PASSWORD: 'hunter22' } });
        expect(await run(['get', '-s', SERVER, shareUrl], ok)).toBe(0);
        expect(await written(ok)).toBe('secret note');
    });

    it('prompts twice for a new password on a terminal', async () => {
        const io = testIo({ stdin: 'x', interactive: true, answers: ['pw-one', 'pw-two'] });
        expect(await run(['put', '-s', SERVER, '-p'], io)).toBe(2);
        expect(io.err()).toContain('do not match');
        expect(server.requests).toHaveLength(0);
    });

    it('never accepts a secret as an argument', async () => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--password=hunter22'], io)).toBe(2);
        expect(await run(['put', '-s', SERVER, '--token', 'cbt_x'], testIo())).toBe(2);
        expect(server.requests).toHaveLength(0);
    });
});

describe('put input limit', () => {
    const LIMIT_MESSAGE = `the input exceeds the capsule limit of ${MAX_CAPSULE_BYTES} bytes`;

    /** A sparse file of `size` bytes in the test directory. */
    async function sizedFile(name: string, size: number): Promise<void> {
        await writeFile(join(cwd, name), '');
        await truncate(join(cwd, name), size);
    }

    /** Ends a refused put: usage exit, the limit named, nothing sent or printed. */
    function expectRefused(io: TestIo, code: number): void {
        expect(code, io.err()).toBe(2);
        expect(io.err()).toContain(LIMIT_MESSAGE);
        expect(io.out()).toBe('');
        expect(server.requests).toHaveLength(0);
    }

    it('put refuses inputs above the capsule limit before reading them', async () => {
        const half = Math.floor(MAX_CAPSULE_BYTES / 2);
        await sizedFile('a.bin', half);
        await sizedFile('b.bin', half);
        await sizedFile('c.bin', half);

        const io = testIo();
        expectRefused(io, await run(['put', '-s', SERVER, 'a.bin', 'b.bin', 'c.bin'], io));
        expect(fsHooks.readStreams).toBe(0);
    });

    it('put stops reading at the capsule limit', async () => {
        const CHUNK = 64 * 1024;
        let consumed = 0;
        async function* endless(): AsyncGenerator<Uint8Array> {
            for (;;) {
                consumed += CHUNK;
                yield new Uint8Array(CHUNK).fill(0x61);
            }
        }
        const io = testIo({ stdin: endless() });
        expectRefused(io, await run(['put', '-s', SERVER], io));
        expect(consumed).toBeGreaterThan(MAX_CAPSULE_BYTES);
        expect(consumed).toBeLessThanOrEqual(MAX_CAPSULE_BYTES + CHUNK);

        // A file and stdin together: the budget spans all inputs.
        await sizedFile('big.bin', MAX_CAPSULE_BYTES - 10);
        const both = testIo({ stdin: 'more than ten bytes' });
        expectRefused(both, await run(['put', '-s', SERVER, 'big.bin', '-'], both));
    });

    it.skipIf(process.platform === 'win32')(
        'put stops reading at the capsule limit of an endless special file',
        async () => {
            const io = testIo();
            expectRefused(io, await run(['put', '-s', SERVER, '/dev/zero'], io));
        },
    );

    it('accepts inputs that fit within the limit together', async () => {
        await sizedFile('a.bin', 1024);
        const io = testIo({ stdin: 'and some stdin' });
        expect(await run(['put', '-s', SERVER, 'a.bin', '-'], io), io.err()).toBe(0);
        expect(server.pastes.size).toBe(1);
    });
});

describe('response limits and timeouts', () => {
    it('response limits and timeouts end with exit 7', async () => {
        // An oversized answer to create is not retried.
        let calls = 0;
        vi.stubGlobal('fetch', async () => {
            calls++;
            return new Response('"' + 'x'.repeat(128 * 1024) + '"', { status: 201 });
        });
        const large = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--json', '--retries', '2'], large)).toBe(7);
        expect(calls).toBe(1);
        expect(JSON.parse(large.out()).error.reason).toBe('response_too_large');
        expect(large.err()).toContain('larger response than any valid answer');
        expect(large.err()).not.toContain('retrying');

        // A create that never answers times out and is replayed unchanged.
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        const realFetch = server.fetch;
        vi.stubGlobal('fetch', async (input: Parameters<typeof fetch>[0], init?: RequestInit) => {
            await realFetch(input, init);
            // The request lands, but its answer never comes.
            return new Promise<Response>(() => {});
        });
        const slow = testIo({ stdin: 'x' });
        const exit = await settleWithTimers(
            run(['put', '-s', SERVER, '--json', '--retries', '1'], slow),
        );
        expect(exit, slow.err()).toBe(7);
        expect(server.requests).toHaveLength(2);
        expect(server.requests[0]!.body).toBe(server.requests[1]!.body);
        expect(server.pastes.size).toBe(1);
        expect(JSON.parse(slow.out()).error.reason).toBe('timeout');
        expect(slow.err()).toContain('timeout, retrying');
        expect(slow.err()).toContain('the server did not answer in time');

        // The same mapping for get, which never retries.
        vi.useRealTimers();
        vi.stubGlobal('fetch', server.fetch);
        const created = await putText('hello');
        const before = server.requests.length;
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        vi.stubGlobal('fetch', () => new Promise<Response>(() => {}));
        const get = testIo();
        expect(await settleWithTimers(run(['get', '--json', created.share_url], get))).toBe(7);
        expect(JSON.parse(get.out()).error.reason).toBe('timeout');
        expect(server.requests).toHaveLength(before);
    });
});

describe('input limit and response bounds, edges', () => {
    it('put refuses a single file one byte above the capsule limit without reading it', async () => {
        await writeFile(join(cwd, 'one.bin'), '');
        await truncate(join(cwd, 'one.bin'), MAX_CAPSULE_BYTES + 1);
        const io = testIo();
        expect(await run(['put', '-s', SERVER, 'one.bin'], io)).toBe(2);
        expect(io.err()).toContain(`${MAX_CAPSULE_BYTES} bytes`);
        expect(io.out()).toBe('');
        expect(server.requests).toHaveLength(0);
        expect(fsHooks.readStreams).toBe(0);
    });

    it('stderr of an oversized answer carries no control characters and no body', async () => {
        vi.stubGlobal(
            'fetch',
            async () =>
                new Response('\u001b[31m' + 'y'.repeat(128 * 1024), {
                    status: 500,
                }),
        );
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--retries', '0'], io)).toBe(7);
        const err = io.err();
        // The message and the delete link of the capsule that may exist.
        const lines = err.replace(/\n$/, '').split('\n');
        expect(lines).toHaveLength(2);
        for (const line of lines) {
            // eslint-disable-next-line no-control-regex
            expect(line).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/);
        }
        // Longer than any run of one letter a random link could plausibly hold.
        expect(err).not.toContain('y'.repeat(32));
    });
});

/** Advances fake timers until `promise` settles, then returns its value. */
async function settleWithTimers<T>(promise: Promise<T>): Promise<T> {
    let settled = false;
    void promise.finally(() => (settled = true));
    while (!settled) await vi.advanceTimersByTimeAsync(10_000);
    return promise;
}

describe('burn after retrieval', () => {
    it('refuses to consume a burn capsule non-interactively without --yes', async () => {
        const created = await putText('once', '--burn');
        expect(created.burn_after_retrieval).toBe(true);

        const io = testIo();
        expect(await run(['get', created.share_url], io)).toBe(8);
        expect(io.err()).toContain('--yes');
        // Still there.
        expect(server.pastes.get(created.id)!.ciphertext).not.toBeNull();

        const yes = testIo();
        expect(await run(['get', created.share_url, '--yes'], yes)).toBe(0);
        expect(await written(yes)).toBe('once');
        expect(yes.err()).toContain('deleted from the server');

        // Afterwards it is gone like any unavailable capsule (the server does not
        // reveal that it was consumed); 4 is reserved for a lost burn race.
        expect(await run(['get', created.share_url, '--yes'], testIo())).toBe(3);
    });

    it('exits with 4 when another retrieval won the burn race', async () => {
        const created = await putText('once', '--burn');
        server.failNextBurnWithRace = true;
        const io = testIo();
        expect(await run(['get', created.share_url, '--yes'], io)).toBe(4);
        expect(io.err()).toContain('already been retrieved');
    });

    it('asks on a terminal and stops on anything but yes', async () => {
        const created = await putText('once', '--burn');
        const no = testIo({ interactive: true, answers: ['n'] });
        expect(await run(['get', created.share_url], no)).toBe(8);

        const yes = testIo({ interactive: true, answers: ['y'] });
        expect(await run(['get', created.share_url], yes)).toBe(0);
    });
});

describe('errors and output', () => {
    it('lists --server for get and --allow-http in the help', async () => {
        const io = testIo();
        expect(await run(['--help'], io)).toBe(0);
        const getSection = io.out().split('get options:')[1]!.split('Common options:')[0]!;
        expect(getSection).toContain('--server URL');
        expect(io.out()).toContain('--allow-http');
        expect(io.out()).toContain('CAPSULEBIN_ALLOW_HTTP=1');
    });

    it('lists the link channels and warns about links as arguments in the help', async () => {
        const io = testIo();
        expect(await run(['--help'], io)).toBe(0);
        const help = io.out();
        expect(help).toContain('--link-file PATH');
        expect(help).toContain('--delete-link-file PATH');
        expect(help).toContain('- to read it from the first line of stdin');
        expect(help).toMatch(/visible to other\s+local users/);
        expect(help).toContain('shell history');
    });

    it('reports failures as JSON on stdout with --json', async () => {
        const io = testIo();
        const missing = `${SERVER}/p/${'A'.repeat(22)}#v1.${'A'.repeat(43)}`;
        expect(await run(['get', missing, '--json'], io)).toBe(3);
        expect(JSON.parse(io.out())).toEqual({
            error: { reason: 'not_available', message: expect.any(String) },
        });
    });

    it('rejects malformed links and swapped link kinds as usage errors', async () => {
        const created = await putText('x');
        expect(await run(['get', created.delete_url], testIo())).toBe(2);
        const io = testIo();
        expect(await run(['delete', created.share_url], io)).toBe(2);
        expect(io.err()).toContain('looks like a share link');
        expect(await run(['get', 'https://x.test/p/short#v1.AAAA'], testIo())).toBe(2);
    });

    it('rejects options that belong to another command', async () => {
        const io = testIo();
        expect(await run(['get', 'x', '--burn'], io)).toBe(2);
        expect(io.err()).toContain('--burn does not apply to "get"');
    });

    it('needs a server for put', async () => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put'], io)).toBe(2);
        expect(io.err()).toContain('CAPSULEBIN_SERVER');
    });

    // Links below such a base would lose the code to the link parser: `delete`
    // would send the delete secret to the base without it.
    it.each([
        'https://paste.example/team/de',
        'https://paste.example/team/de/',
        'https://paste.example/team/de/.',
        'https://paste.example/team/x/../de',
        'https://paste.example/team/%2e/en',
    ])('refuses the server %s, which ends in a language code', async (base) => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '--server', base], io)).toBe(2);
        expect(io.err()).toContain('--server must not end in the language code');
        expect(server.requests).toHaveLength(0);

        const fromEnv = testIo({ stdin: 'x', env: { CAPSULEBIN_SERVER: base } });
        expect(await run(['put'], fromEnv)).toBe(2);
        expect(fromEnv.err()).toContain('language code');
        expect(server.requests).toHaveLength(0);
    });

    // The API path is appended to the base as text: behind a query or fragment
    // the request, and a creation token with it, would go to the base's path.
    it.each([
        'https://paste.example/team?lang=/de',
        'https://paste.example/team#/de',
        'https://paste.example/team?',
        'https://paste.example/team#',
        'https://paste.example/team?x=1/',
        'https://paste.example/team#x//',
        'https://paste.example?',
    ])('refuses the server %s, which has a query or fragment', async (base) => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '--server', base], io)).toBe(2);
        expect(io.err()).toContain('--server must not contain a query or fragment');
        expect(server.requests).toHaveLength(0);

        const fromEnv = testIo({ stdin: 'x', env: { CAPSULEBIN_SERVER: base } });
        expect(await run(['put'], fromEnv)).toBe(2);
        expect(fromEnv.err()).toContain('must not contain a query or fragment');
        expect(server.requests).toHaveLength(0);
    });
});

describe('transport', () => {
    it.each([
        'http://paste.example',
        'http://paste.localhost',
        'http://localhost.example',
        'http://10.0.0.1',
        'http://192.168.1.10:8080',
        'http://[::2]',
        'http://128.0.0.1',
        // Loopback is exactly localhost, 127.0.0.0/8 and [::1]; look-alikes and
        // other local addresses need the opt-in.
        'http://127.0.0.1.nip.io',
        'http://[::ffff:127.0.0.1]',
        'http://0.0.0.0:8081',
        'http://localhost.:8081',
    ])('refuses plain HTTP to %s without opt-in, before any request', async (base) => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '--server', base], io)).toBe(2);
        expect(io.err()).toContain('--allow-http');
        expect(io.err()).toContain('CAPSULEBIN_ALLOW_HTTP');

        const fromEnv = testIo({ stdin: 'x', env: { CAPSULEBIN_SERVER: base } });
        expect(await run(['put'], fromEnv)).toBe(2);
        expect(server.requests).toHaveLength(0);

        const allowed = testIo({ stdin: 'x' });
        expect(await run(['put', '--server', base, '--allow-http'], allowed), allowed.err()).toBe(
            0,
        );
    });

    it.each([
        'http://localhost:8081',
        'http://LOCALHOST:8081/',
        'http://127.0.0.1:8081',
        'http://127.1.2.3:8081',
        'http://[::1]:8081',
        'https://paste.example',
    ])('talks to %s without opt-in', async (base) => {
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '--server', base, '--json'], io), io.err()).toBe(0);
        const created = JSON.parse(io.out()) as { share_url: string; delete_url: string };

        const getIo = testIo();
        expect(await run(['get', created.share_url], getIo), getIo.err()).toBe(0);
        expect(await written(getIo)).toBe('x');
        expect(await run(['delete', created.delete_url], testIo())).toBe(0);
    });

    it('refuses an http link for get and delete unless HTTP is allowed', async () => {
        const io = testIo({ stdin: 'over http' });
        expect(await run(['put', '-s', 'http://capsule.test', '--allow-http', '--json'], io)).toBe(
            0,
        );
        const created = JSON.parse(io.out()) as {
            id: string;
            share_url: string;
            delete_url: string;
        };
        const before = server.requests.length;

        for (const [command, link] of [
            ['get', created.share_url],
            ['delete', created.delete_url],
        ] as const) {
            const refused = testIo();
            expect(await run([command, link], refused)).toBe(2);
            expect(refused.err()).toContain('--allow-http');
            expect(refused.err()).toContain('CAPSULEBIN_ALLOW_HTTP');
            expect(refused.err()).not.toContain('#v1.');

            const typo = testIo({ env: { CAPSULEBIN_ALLOW_HTTP: 'true' } });
            expect(await run([command, link], typo)).toBe(2);
            expect(typo.err()).toContain('CAPSULEBIN_ALLOW_HTTP');
        }
        expect(server.requests).toHaveLength(before);

        const viaEnv = testIo({ env: { CAPSULEBIN_ALLOW_HTTP: '1' } });
        expect(await run(['get', created.share_url], viaEnv), viaEnv.err()).toBe(0);
        expect(await written(viaEnv)).toBe('over http');
        const viaFlag = testIo();
        expect(await run(['get', '--allow-http', created.share_url], viaFlag), viaFlag.err()).toBe(
            0,
        );
        expect(await written(viaFlag)).toBe('over http');
        const deleted = testIo();
        expect(
            await run(['delete', created.delete_url, '--allow-http'], deleted),
            deleted.err(),
        ).toBe(0);
        expect(server.pastes.has(created.id)).toBe(false);

        const second = testIo({ stdin: 'x' });
        expect(
            await run(['put', '-s', 'http://capsule.test', '--allow-http', '--json'], second),
        ).toBe(0);
        const other = JSON.parse(second.out()) as { id: string; delete_url: string };
        const deletedViaEnv = testIo({ env: { CAPSULEBIN_ALLOW_HTTP: '1' } });
        expect(await run(['delete', other.delete_url], deletedViaEnv), deletedViaEnv.err()).toBe(0);
        expect(server.pastes.has(other.id)).toBe(false);
    });

    it('lets put talk plain HTTP to a remote host with CAPSULEBIN_ALLOW_HTTP=1', async () => {
        const io = testIo({ stdin: 'via env', env: { CAPSULEBIN_ALLOW_HTTP: '1' } });
        expect(await run(['put', '--server', 'http://paste.example', '--json'], io), io.err()).toBe(
            0,
        );
        const created = JSON.parse(io.out()) as { share_url: string };
        expect(created.share_url.startsWith('http://paste.example/p/')).toBe(true);
        expect(server.requests).toHaveLength(1);
        expect(server.requests[0]!.url.startsWith('http://paste.example/api/v1/pastes')).toBe(true);
    });

    it.each(['true', 'yes', '0'])(
        'refuses CAPSULEBIN_ALLOW_HTTP=%s as a usage error naming the variable',
        async (value) => {
            const io = testIo({ stdin: 'x', env: { CAPSULEBIN_ALLOW_HTTP: value } });
            expect(await run(['put', '-s', 'http://paste.example'], io)).toBe(2);
            expect(io.err()).toContain('CAPSULEBIN_ALLOW_HTTP');
            expect(io.err()).not.toContain('plain HTTP');
            expect(server.requests).toHaveLength(0);
        },
    );

    it.each([307, 308])(
        'does not retry a %i on create and does not show where it pointed',
        async (status) => {
            let calls = 0;
            vi.stubGlobal('fetch', async () => {
                calls++;
                return new Response(null, {
                    status,
                    headers: { Location: 'https://elsewhere.test/collect' },
                });
            });
            const io = testIo({ stdin: 'x' });
            expect(await run(['put', '-s', SERVER, '--json'], io)).toBe(7);
            expect(calls).toBe(1);
            expect(JSON.parse(io.out()).error.reason).toBe('redirect_refused');
            expect(io.out()).not.toContain('elsewhere.test');
            expect(io.err()).toContain('redirect, which was not followed');
            expect(io.err()).not.toContain('elsewhere.test');
            expect(io.err()).not.toContain('retrying');
        },
    );
});

/** Routes requests below `/team` to the fake server, as a server under a sub-path. */
function serveBelowTeam(): void {
    vi.stubGlobal('fetch', (input: Parameters<typeof fetch>[0], init?: RequestInit) =>
        server.fetch(String(input).replace('/team/api/', '/api/'), init),
    );
}

describe('get --server and CAPSULEBIN_PASSWORD', () => {
    async function protectedCapsule(base = SERVER): Promise<string> {
        await writeFile(join(cwd, 'pw'), 'hunter22\n');
        const io = testIo({ stdin: 'secret note' });
        expect(await run(['put', '-s', base, '--password-file', 'pw'], io), io.err()).toBe(0);
        return io.out().trim();
    }

    const retrieves = () => server.requests.filter((r) => r.url.endsWith('/retrieve'));
    const env = { CAPSULEBIN_PASSWORD: 'hunter22' };

    it.each([
        ['trailing slash', `${SERVER}/`],
        ['upper-case host', 'https://CAPSULE.TEST'],
        ['default port', `${SERVER}:443`],
    ])('uses the environment password when --server matches (%s)', async (_, anchor) => {
        const shareUrl = await protectedCapsule();
        const io = testIo({ env });
        expect(await run(['get', '--server', anchor, shareUrl], io), io.err()).toBe(0);
        expect(await written(io)).toBe('secret note');
    });

    it('uses the environment password for a link with a language segment', async () => {
        const shareUrl = (await protectedCapsule()).replace(`${SERVER}/`, `${SERVER}/de/`);
        const io = testIo({ env });
        expect(await run(['get', '-s', SERVER, shareUrl], io), io.err()).toBe(0);
        expect(await written(io)).toBe('secret note');
    });

    it('takes CAPSULEBIN_SERVER as the trusted server without --server', async () => {
        const shareUrl = await protectedCapsule();
        const io = testIo({ env: { ...env, CAPSULEBIN_SERVER: `${SERVER}/` } });
        expect(await run(['get', shareUrl], io), io.err()).toBe(0);
    });

    it('compares the sub-path of the base', async () => {
        serveBelowTeam();
        const shareUrl = await protectedCapsule(`${SERVER}/team`);
        expect(shareUrl.startsWith(`${SERVER}/team/p/`)).toBe(true);

        const io = testIo({ env });
        expect(await run(['get', '-s', `${SERVER}/team/`, shareUrl], io), io.err()).toBe(0);

        const other = testIo({ env });
        expect(await run(['get', '-s', `${SERVER}/other`, shareUrl], other)).toBe(8);
        const parent = testIo({ env });
        expect(await run(['get', '-s', SERVER, shareUrl], parent)).toBe(8);
    });

    it.each([
        ['no anchor', []],
        ['another host', ['-s', 'https://other.test']],
        ['another port', ['-s', 'https://capsule.test:8443']],
        ['another scheme', ['-s', 'http://capsule.test', '--allow-http']],
        ['another sub-path', ['-s', `${SERVER}/team`]],
    ])('stops non-interactively with %s, before any retrieve', async (_, args) => {
        const shareUrl = await protectedCapsule();
        const io = testIo({ env });
        expect(await run(['get', ...args, shareUrl, '--json'], io)).toBe(8);
        expect(JSON.parse(io.out()).error.reason).toBe('password_origin_mismatch');
        expect(io.err()).toContain(`this link's server is ${SERVER}`);
        expect(io.err()).toContain('--server');
        expect(io.err()).not.toContain('hunter22');
        expect(retrieves()).toHaveLength(0);
    });

    it('names the given --server in the mismatch message', async () => {
        const shareUrl = await protectedCapsule();
        const io = testIo({ env });
        expect(await run(['get', '-s', 'https://other.test/', shareUrl], io)).toBe(8);
        expect(io.err()).toContain('--server is https://other.test');
    });

    it('prompts on a terminal instead of using the environment password', async () => {
        const shareUrl = await protectedCapsule();
        // The environment password is right, the prompted one wrong: failing
        // proves the environment password was not used.
        const io = testIo({ env, interactive: true, answers: ['not-it'] });
        expect(await run(['get', '-s', 'https://other.test', shareUrl], io)).toBe(3);
        expect(io.err()).toContain('the environment password is not used');
        expect(io.prompts).toEqual(['Password: ']);
        expect(retrieves()).toHaveLength(1);

        const right = testIo({
            env: { CAPSULEBIN_PASSWORD: 'wrong' },
            interactive: true,
            answers: ['hunter22'],
        });
        expect(await run(['get', shareUrl], right)).toBe(0);
        expect(await written(right)).toBe('secret note');
        expect(retrieves()).toHaveLength(2);
    });

    it('uses --password-file for any link', async () => {
        const shareUrl = await protectedCapsule();
        const io = testIo({ env: { CAPSULEBIN_PASSWORD: 'wrong' } });
        expect(
            await run(['get', '-s', 'https://other.test', '--password-file', 'pw', shareUrl], io),
            io.err(),
        ).toBe(0);
        expect(await run(['get', '--password-file', 'pw', shareUrl], testIo())).toBe(0);
    });

    it('ignores an unusable CAPSULEBIN_SERVER', async () => {
        const created = await putText('open');
        for (const value of ['not a url', 'https://paste.example/de', 'http://paste.example']) {
            const io = testIo({ env: { CAPSULEBIN_SERVER: value } });
            expect(await run(['get', created.share_url], io), io.err()).toBe(0);
            expect(await written(io)).toBe('open');
        }

        const shareUrl = await protectedCapsule();
        const io = testIo({ env: { ...env, CAPSULEBIN_SERVER: `${SERVER}/de` } });
        expect(await run(['get', shareUrl], io)).toBe(8);
    });

    it.each([
        ['ftp://capsule.test', 'http(s) URL'],
        ['https://capsule.test/de', 'language code'],
        ['https://capsule.test/?x', 'query or fragment'],
        ['http://capsule.test', '--allow-http'],
    ])('refuses an explicit --server %s as a usage error', async (anchor, message) => {
        const created = await putText('x');
        const before = server.requests.length;
        const io = testIo();
        expect(await run(['get', '--server', anchor, created.share_url], io)).toBe(2);
        expect(io.err()).toContain(message);
        expect(server.requests).toHaveLength(before);
    });

    it('ends with invalid_capsule, without a prompt, on a header below the KDF floor', async () => {
        const shareUrl = await protectedCapsule();
        const stored = [...server.pastes.values()][0]!;
        const header = decodeUtf8(fromBase64url(stored.publicHeader));
        const weak = header.replace(/"iterations":\d+/, '"iterations":1000');
        expect(weak).not.toBe(header);
        stored.publicHeader = toBase64url(encodeUtf8(weak));

        const io = testIo({ env, interactive: true });
        expect(await run(['get', '-s', SERVER, shareUrl, '--json'], io)).toBe(5);
        expect(JSON.parse(io.out()).error.reason).toBe('invalid_capsule');
        expect(io.prompts).toHaveLength(0);
        expect(retrieves()).toHaveLength(0);
    });
});

describe('new passwords', () => {
    it('refuses a password file with only whitespace, naming the file', async () => {
        await writeFile(join(cwd, 'blank'), ' \t \nsecond line\n');
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--password-file', 'blank'], io)).toBe(2);
        expect(io.err()).toContain('password file blank contains only whitespace');
        expect(server.requests).toHaveLength(0);
    });

    it('refuses CAPSULEBIN_PASSWORD with only whitespace, naming the variable', async () => {
        const io = testIo({ stdin: 'x', env: { CAPSULEBIN_PASSWORD: '   ' } });
        expect(await run(['put', '-s', SERVER, '--password'], io)).toBe(2);
        expect(io.err()).toContain('CAPSULEBIN_PASSWORD contains only whitespace');
        expect(server.requests).toHaveLength(0);
    });

    it('keeps surrounding spaces of a password', async () => {
        await writeFile(join(cwd, 'spaced'), ' secret \n');
        await writeFile(join(cwd, 'trimmed'), 'secret\n');
        const io = testIo({ stdin: 'spaced' });
        expect(await run(['put', '-s', SERVER, '--password-file', 'spaced'], io)).toBe(0);
        const shareUrl = io.out().trim();

        expect(await run(['get', '--password-file', 'trimmed', shareUrl], testIo())).toBe(3);
        const ok = testIo();
        expect(await run(['get', '--password-file', 'spaced', shareUrl], ok)).toBe(0);
        expect(await written(ok)).toBe('spaced');
    });
});

describe('empty path options', () => {
    it('put refuses an empty path before any request, without a fallback', async () => {
        server.creationToken = 'cbt_from_env';
        const env = { CAPSULEBIN_TOKEN: 'cbt_from_env', CAPSULEBIN_PASSWORD: 'hunter22' };
        const cases: [string[], string][] = [
            [['--password-file', ''], '--password-file'],
            [['-p', '--password-file', ''], '--password-file'],
            [['--token-file', ''], '--token-file'],
            [['--delete-link-file', ''], '--delete-link-file'],
        ];
        for (const [args, option] of cases) {
            const io = testIo({ stdin: 'x', env, interactive: true });
            expect(await run(['put', '-s', SERVER, ...args], io), args.join(' ')).toBe(2);
            expect(io.err()).toContain(`${option} needs a path`);
            expect(io.out()).toBe('');
            expect(io.prompts).toEqual([]);
        }
        expect(server.requests).toHaveLength(0);
        expect(await readdir(cwd)).toEqual([]);
    });

    it('get and delete refuse an empty path before any request, without a fallback', async () => {
        await writeFile(join(cwd, 'pw'), 'hunter22\n');
        const created = await putText('secret note', '--password-file', 'pw');
        const before = server.requests.length;
        const entries = await readdir(cwd);

        const env = { CAPSULEBIN_PASSWORD: 'hunter22', CAPSULEBIN_SERVER: SERVER };
        const cases: [string[], string][] = [
            [['get', '--password-file', '', created.share_url], '--password-file'],
            [['get', '-o', '', created.share_url], '--output'],
            [['get', '--link-file', ''], '--link-file'],
            [['delete', '--link-file', ''], '--link-file'],
        ];
        for (const [argv, option] of cases) {
            const io = testIo({ env, interactive: true });
            expect(await run(argv, io), argv.join(' ')).toBe(2);
            expect(io.err()).toContain(`${option} needs a path`);
            expect(io.out()).toBe('');
            expect(io.prompts).toEqual([]);
        }
        expect(server.requests).toHaveLength(before);
        expect(await readdir(cwd)).toEqual(entries);
        expect(server.pastes.has(created.id)).toBe(true);
    });
});

describe('first line of a link, token or password source', () => {
    const LIMIT = 16_384;

    /** `link` with a query that brings it to exactly `length` bytes. */
    function paddedLink(link: string, length: number): string {
        const [path, fragment] = link.split('#') as [string, string];
        const padded = `${path}?${'q'.repeat(length - link.length - 1)}#${fragment}`;
        expect(padded).toHaveLength(length);
        return padded;
    }

    /** Ends a refused command: usage exit, the source named, none of the line shown. */
    function expectTooLong(io: TestIo, code: number, source: string, content: string): void {
        expect(code, io.err()).toBe(2);
        expect(io.err()).toContain(`the first line of the ${source} exceeds ${LIMIT} bytes`);
        expect(io.err()).not.toContain(content);
        expect(io.out()).toBe('');
    }

    it('reads the link from a stdin that stays open after its first line', async () => {
        const created = await putText('from an open pipe');
        async function* open(): AsyncGenerator<Uint8Array> {
            yield new TextEncoder().encode(created.share_url + '\n');
            await new Promise(() => {});
        }

        const io = testIo({ stdin: open() });
        expect(await run(['get', '-'], io), io.err()).toBe(0);
        expect(await written(io)).toBe('from an open pipe');
    });

    it('reads a link split over several chunks of stdin', async () => {
        const created = await putText('in pieces');
        const bytes = new TextEncoder().encode(created.share_url + '\r\nignored\n');
        async function* pieces(): AsyncGenerator<Uint8Array> {
            yield bytes.subarray(0, 20);
            yield bytes.subarray(20);
        }
        const io = testIo({ stdin: pieces() });
        expect(await run(['get', '-'], io), io.err()).toBe(0);
        expect(await written(io)).toBe('in pieces');
    });

    it.each([
        ['\\n', '\n'],
        ['\\r\\n', '\r\nsecond line\n'],
        ['no line ending', ''],
    ])('accepts a link of 16384 bytes and refuses one of 16385 (%s)', async (_, ending) => {
        const created = await putText('long link');
        const before = server.requests.length;

        await writeFile(join(cwd, 'long'), paddedLink(created.share_url, LIMIT + 1) + ending);
        const long = testIo();
        expectTooLong(
            long,
            await run(['get', '--link-file', 'long'], long),
            'link from file long',
            'qqqq',
        );
        const piped = testIo({ stdin: paddedLink(created.delete_url, LIMIT + 1) + ending });
        expectTooLong(piped, await run(['delete', '-'], piped), 'link from stdin', 'qqqq');
        for (const io of [long, piped]) {
            expect(io.err()).not.toContain('#v1.');
            expect(io.err()).not.toContain(created.id);
        }
        expect(server.requests).toHaveLength(before);

        await writeFile(join(cwd, 'fits'), paddedLink(created.share_url, LIMIT) + ending);
        const fits = testIo();
        expect(await run(['get', '--link-file', 'fits'], fits), fits.err()).toBe(0);
        expect(await written(fits)).toBe('long link');
        const del = testIo({ stdin: paddedLink(created.delete_url, LIMIT) + ending });
        expect(await run(['delete', '-'], del), del.err()).toBe(0);
        expect(server.pastes.has(created.id)).toBe(false);
    });

    it.each([
        ['\\n', '\n'],
        ['\\r\\n', '\r\nsecond line\n'],
        ['no line ending', ''],
    ])('accepts a token of 16384 bytes and refuses one of 16385 (%s)', async (_, ending) => {
        const token = 't'.repeat(LIMIT);
        server.creationToken = token;
        await writeFile(join(cwd, 'long'), token + 't' + ending);
        await writeFile(join(cwd, 'fits'), token + ending);

        const long = testIo({ stdin: 'x', env: { CAPSULEBIN_TOKEN: token } });
        expectTooLong(
            long,
            await run(['put', '-s', SERVER, '--token-file', 'long'], long),
            'token from file long',
            'tttt',
        );
        expect(server.requests).toHaveLength(0);

        const fits = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--token-file', 'fits'], fits), fits.err()).toBe(0);
        expect(server.requests[0]!.headers.get('Authorization')).toBe(`Bearer ${token}`);
    });

    it.each([
        ['\\n', '\n'],
        ['\\r\\n', '\r\nsecond line\n'],
        ['no line ending', ''],
    ])('accepts a password of 16384 bytes and refuses one of 16385 (%s)', async (_, ending) => {
        const password = 'p'.repeat(LIMIT);
        await writeFile(join(cwd, 'long'), password + 'p' + ending);
        await writeFile(join(cwd, 'fits'), password + ending);
        await writeFile(join(cwd, 'exact'), password);

        const long = testIo({ stdin: 'x' });
        expectTooLong(
            long,
            await run(['put', '-s', SERVER, '--password-file', 'long'], long),
            'password from file long',
            'pppp',
        );
        expect(server.requests).toHaveLength(0);

        const put = testIo({ stdin: 'long password' });
        expect(await run(['put', '-s', SERVER, '--password-file', 'fits'], put), put.err()).toBe(0);
        const shareUrl = put.out().trim();
        const before = server.requests.length;

        const refused = testIo({ env: { CAPSULEBIN_PASSWORD: password }, interactive: true });
        expectTooLong(
            refused,
            await run(['get', '-s', SERVER, '--password-file', 'long', shareUrl], refused),
            'password from file long',
            'pppp',
        );
        expect(refused.prompts).toEqual([]);
        expect(server.requests.slice(before).filter((r) => r.url.endsWith('/retrieve'))).toEqual(
            [],
        );

        // The line ending is not part of the password: the bare file opens it.
        const get = testIo();
        expect(await run(['get', '--password-file', 'exact', shareUrl], get), get.err()).toBe(0);
        expect(await written(get)).toBe('long password');
    });

    it.skipIf(process.platform === 'win32')(
        'stops at the limit of an endless special file',
        async () => {
            const created = await putText('x');
            const before = server.requests.length;

            const put = testIo({ stdin: 'x' });
            expectTooLong(
                put,
                await run(['put', '-s', SERVER, '--password-file', '/dev/zero'], put),
                'password from file /dev/zero',
                '\u0000',
            );
            const get = testIo();
            expectTooLong(
                get,
                await run(['get', '--link-file', '/dev/zero'], get),
                'link from file /dev/zero',
                '\u0000',
            );
            expect(server.requests).toHaveLength(before);
            expect(server.pastes.has(created.id)).toBe(true);
        },
    );

    it('reads no more than the limit plus one chunk', async () => {
        const CHUNK = 64 * 1024;
        let consumed = 0;
        async function* endless(): AsyncGenerator<Uint8Array> {
            for (;;) {
                consumed += CHUNK;
                yield new Uint8Array(CHUNK).fill(0x61);
            }
        }
        expect(await readFirstLine(endless(), LIMIT)).toBeNull();
        expect(consumed).toBe(CHUNK);

        consumed = 0;
        const io = testIo({ stdin: endless() });
        expectTooLong(io, await run(['get', '-'], io), 'link from stdin', 'aaaa');
        expect(consumed).toBe(CHUNK);
    });
});

describe('CAPSULEBIN_TOKEN and CAPSULEBIN_SERVER', () => {
    const A = SERVER;
    const B = 'https://other.test';
    const TOKEN = 'cbt_for_a';

    /** Runs a put and returns its io and the one create request it sent. */
    async function create(env: Record<string, string>, ...args: string[]) {
        const before = server.requests.length;
        const io = testIo({ stdin: 'x', env: { CAPSULEBIN_TOKEN: TOKEN, ...env } });
        expect(await run(['put', ...args], io), io.err()).toBe(0);
        const sent = server.requests.slice(before);
        expect(sent).toHaveLength(1);
        return { io, request: sent[0]! };
    }

    it.each([[[] as string[]], [['--json']]])(
        'withholds the token from a --server other than CAPSULEBIN_SERVER (%j)',
        async (mode) => {
            const { io, request } = await create({ CAPSULEBIN_SERVER: A }, '--server', B, ...mode);
            expect(request.url).toBe(`${B}/api/v1/pastes`);
            expect(request.headers.get('Authorization')).toBeNull();
            expect(io.err()).toContain('CAPSULEBIN_TOKEN');
            expect(io.err()).toContain(A);
            expect(io.err()).toContain(B);
            expect(io.err()).not.toContain(TOKEN);
            expect(io.out()).not.toContain('CAPSULEBIN_TOKEN');
        },
    );

    it.each([
        ['without --server', []],
        ['with the same --server and a trailing slash', ['--server', `${A}/`]],
    ])('sends the token to CAPSULEBIN_SERVER (%s)', async (_, args) => {
        const { io, request } = await create({ CAPSULEBIN_SERVER: A }, ...args);
        expect(request.url).toBe(`${A}/api/v1/pastes`);
        expect(request.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
        expect(io.err()).not.toContain('CAPSULEBIN_TOKEN');
    });

    it('sends the token to any --server without CAPSULEBIN_SERVER', async () => {
        const { io, request } = await create({}, '--server', B);
        expect(request.url).toBe(`${B}/api/v1/pastes`);
        expect(request.headers.get('Authorization')).toBe(`Bearer ${TOKEN}`);
        expect(io.err()).not.toContain('CAPSULEBIN_TOKEN');
    });

    it('sends the token of --token-file to any server', async () => {
        await writeFile(join(cwd, 'token'), 'cbt_for_b\n');
        const { io, request } = await create(
            { CAPSULEBIN_SERVER: A },
            ...['--server', B, '--token-file', 'token'],
        );
        expect(request.url).toBe(`${B}/api/v1/pastes`);
        expect(request.headers.get('Authorization')).toBe('Bearer cbt_for_b');
        expect(io.err()).not.toContain('CAPSULEBIN_TOKEN');
    });

    it('withholds the token when CAPSULEBIN_SERVER is no usable URL', async () => {
        const { io, request } = await create({ CAPSULEBIN_SERVER: 'not a url' }, '--server', B);
        expect(request.headers.get('Authorization')).toBeNull();
        expect(io.err()).toContain('CAPSULEBIN_TOKEN');
        expect(io.err()).toContain(B);
        expect(io.err()).not.toContain(TOKEN);
    });

    it('ends with exit 6 when the server needs the withheld token', async () => {
        server.creationToken = TOKEN;
        const io = testIo({
            stdin: 'x',
            env: { CAPSULEBIN_TOKEN: TOKEN, CAPSULEBIN_SERVER: A },
        });
        expect(await run(['put', '--server', B], io)).toBe(6);
        expect(io.err()).toContain('CAPSULEBIN_TOKEN');
    });
});

describe('parseDuration', () => {
    it.each([
        ['90s', 90],
        ['30m', 1800],
        ['12h', 43_200],
        ['7d', 604_800],
        ['2w', 1_209_600],
        ['never', null],
    ])('%s', (text, seconds) => {
        expect(parseDuration(text)).toBe(seconds);
    });

    it.each(['', '7', 'd', '0d', '-1d', '1.5h', '7days'])('rejects %j', (text) => {
        expect(() => parseDuration(text)).toThrow();
    });
});

describe('buildItem', () => {
    const bytes = (text: string) => new TextEncoder().encode(text);

    it.each([
        ['README.md', 'markdown', undefined],
        ['script.py', 'code', 'python'],
        ['Cargo.toml', 'code', 'ini'],
        ['notes.txt', 'plain', undefined],
        ['Makefile', 'plain', undefined],
    ])('infers %s as %s', (name, presentation, language) => {
        const item = buildItem({ name, bytes: bytes('content') }, {});
        expect(item.presentation).toBe(presentation);
        expect(item.language).toBe(language);
    });

    it('treats non-UTF-8 or NUL-containing data as a download', () => {
        expect(
            buildItem({ name: 'a.txt', bytes: new Uint8Array([0xff, 0xfe]) }, {}).presentation,
        ).toBe('download');
        expect(
            buildItem({ name: 'a.txt', bytes: new Uint8Array([65, 0, 66]) }, {}).presentation,
        ).toBe('download');
    });

    it('honours --as and --lang, and refuses binary data as text', () => {
        expect(buildItem({ name: 'x', bytes: bytes('a') }, { lang: 'sql' })).toMatchObject({
            presentation: 'code',
            language: 'sql',
        });
        expect(
            buildItem({ name: 'x.md', bytes: bytes('a') }, { as: 'download' }).presentation,
        ).toBe('download');
        expect(() =>
            buildItem({ name: 'x', bytes: new Uint8Array([0xff]) }, { as: 'plain' }),
        ).toThrow(/use --as file/);
    });
});

describe('safeFileName', () => {
    it.each([
        ['notes.md', 'notes.md'],
        ['../../etc/passwd', 'passwd'],
        ['C:\\Windows\\evil.exe', 'evil.exe'],
        ['..', 'item-1'],
        ['', 'item-1'],
        ['a\u0000b\nc', 'a_b_c'],
        ['what?.txt', 'what_.txt'],
        ['a\u009bb\u0080c\u009f', 'a_b_c_'],
        ['.bash_profile', '_bash_profile'],
        ['..envrc', '__envrc'],
        ['...', 'item-1'],
        [' .npmrc ', '_npmrc'],
        ['dir/.envrc', '_envrc'],
        ['a.b', 'a.b'],
    ])('%j → %j', (name, expected) => {
        expect(safeFileName(name, 0)).toBe(expected);
    });

    it('leaves no C0, DEL or C1 character in a name', () => {
        // eslint-disable-next-line no-control-regex
        const controls = /[\u0000-\u001f\u007f-\u009f]/;
        expect(safeFileName('\u009b31m\u001b[2Jx.txt', 0)).not.toMatch(controls);
    });

    const bytes = (text: string) => Buffer.byteLength(text, 'utf8');
    // NAME_MAX (255) less the longest suffix ` (999)`.
    const LIMIT = 249;

    it('shortens a long name to leave room for a suffix, keeping a short extension', () => {
        const name = safeFileName(`${'n'.repeat(300)}.txt`, 0);
        expect(name).toBe(`${'n'.repeat(LIMIT - 4)}.txt`);
        expect(bytes(name)).toBe(LIMIT);
        expect(safeFileName('n'.repeat(LIMIT), 0)).toBe('n'.repeat(LIMIT));

        const longExtension = safeFileName(`a.${'b'.repeat(300)}`, 0);
        expect(longExtension).toBe(`a.${'b'.repeat(LIMIT - 2)}`);
    });

    it.each([
        ['ä'.repeat(200) + '.md', '.md'],
        ['😀'.repeat(100), ''],
        ['a' + '漢'.repeat(120) + '.txt', '.txt'],
    ])('cuts %j only between code points', (input, extension) => {
        const name = safeFileName(input, 0);
        expect(bytes(name)).toBeLessThanOrEqual(LIMIT);
        expect(bytes(name)).toBeGreaterThan(LIMIT - 4);
        expect(name.endsWith(extension)).toBe(true);
        // No lone surrogate and no broken sequence: the name survives UTF-8.
        expect(Buffer.from(name, 'utf8').toString('utf8')).toBe(name);
        expect(input.startsWith(name.slice(0, name.length - extension.length))).toBe(true);
    });
});

describe('file output', () => {
    const posix = process.platform !== 'win32';
    const mode = async (path: string) => (await stat(path)).mode & 0o777;

    /**
     * Puts files (created with the given contents in a directory outside
     * `cwd`) into one capsule and returns the share link.
     */
    async function putFiles(files: [string, string | Uint8Array][]): Promise<string> {
        const sources = await mkdtemp(join(tmpdir(), 'capsulebin-src-'));
        const names: string[] = [];
        for (const [index, [name, content]] of files.entries()) {
            const dir = join(sources, String(index));
            await mkdir(dir);
            await writeFile(join(dir, name), content);
            names.push(join(dir, name));
        }
        const io = testIo();
        expect(await run(['put', '-s', SERVER, ...names], io), io.err()).toBe(0);
        return io.out().trim();
    }

    /** The directory a capsule gets without `-o`/`--here`, on 2026-01-05. */
    function capsuleDir(link: string): string {
        const id = /\/p\/([A-Za-z0-9_-]{22})#/.exec(link)![1]!;
        return `capsule-2026-01-05-${id.slice(0, 8)}`;
    }

    beforeEach(() => {
        // Late in the evening, local time: the directory name uses the local date.
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(new Date(2026, 0, 5, 23, 59));
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    async function withUmask<T>(umask: number, body: () => Promise<T>): Promise<T> {
        const previous = process.umask(umask);
        try {
            return await body();
        } finally {
            process.umask(previous);
        }
    }

    it.skipIf(!posix)('writes owner-only files and creates owner-only directories', async () => {
        await withUmask(0o000, async () => {
            const multi = await putFiles([
                ['a.txt', 'a'],
                ['b.txt', 'b'],
            ]);
            const single = await putFiles([['img.bin', new Uint8Array([0, 159, 146, 150])]]);

            const intoNew = testIo();
            expect(await run(['get', multi], intoNew), intoNew.err()).toBe(0);
            const here = testIo();
            expect(await run(['get', multi, '--here'], here), here.err()).toBe(0);
            const created = join(cwd, 'new', 'out');
            const intoDir = testIo();
            expect(await run(['get', multi, '-o', created], intoDir), intoDir.err()).toBe(0);
            const one = testIo();
            expect(await run(['get', single], one), one.err()).toBe(0);

            const written = [intoNew, here, intoDir, one].flatMap(wrotePaths);
            expect(written).toHaveLength(7);
            for (const path of written) expect(await mode(path), path).toBe(0o600);
            expect(await mode(join(cwd, capsuleDir(multi)))).toBe(0o700);
            expect(await mode(created)).toBe(0o700);
            expect(await mode(join(cwd, 'new'))).toBe(0o700);

            const existing = join(cwd, 'existing');
            await mkdir(existing);
            await chmod(existing, 0o755);
            expect(await run(['get', multi, '-o', existing], testIo())).toBe(0);
            expect(await mode(existing)).toBe(0o755);
            expect(await mode(join(existing, 'a.txt'))).toBe(0o600);
        });
    });

    it('never opens, follows or replaces an existing entry at the target name', async () => {
        const link = await putFiles([
            ['note.txt', 'first'],
            ['note.txt', 'second'],
        ]);
        const out = join(cwd, 'out');
        await mkdir(out);
        const outside = join(cwd, 'outside.txt');
        await writeFile(outside, 'keep me');
        const dangling = join(cwd, 'nowhere.txt');
        await writeFile(join(out, 'note.txt'), 'old');
        await symlink(outside, join(out, 'note (1).txt'));
        await symlink(dangling, join(out, 'note (2).txt'));
        await mkdir(join(out, 'note (3).txt'));

        const io = testIo();
        expect(await run(['get', link, '-o', out, '--json'], io), io.err()).toBe(0);
        const paths = JSON.parse(io.out()).items.map((i: { path: string }) => i.path);
        expect(paths).toEqual([join(out, 'note (4).txt'), join(out, 'note (5).txt')]);
        expect(await readFile(paths[0], 'utf8')).toBe('first');
        expect(await readFile(paths[1], 'utf8')).toBe('second');

        expect(await readFile(join(out, 'note.txt'), 'utf8')).toBe('old');
        expect(await readlink(join(out, 'note (1).txt'))).toBe(outside);
        expect(await readFile(outside, 'utf8')).toBe('keep me');
        expect(await readlink(join(out, 'note (2).txt'))).toBe(dangling);
        await expect(access(dangling)).rejects.toThrow();
        expect((await lstat(join(out, 'note (3).txt'))).isDirectory()).toBe(true);
        expect(await readdir(join(out, 'note (3).txt'))).toEqual([]);
        expect((await readdir(out)).sort()).toEqual([
            'note (1).txt',
            'note (2).txt',
            'note (3).txt',
            'note (4).txt',
            'note (5).txt',
            'note.txt',
        ]);
    });

    it('has no --force, -f or --raw and fails naming the item when no name is free', async () => {
        const link = await putFiles([['note.txt', 'x']]);
        for (const flag of ['--force', '-f', '--raw']) {
            const io = testIo();
            expect(await run(['get', link, flag], io), flag).toBe(2);
        }
        const help = testIo();
        expect(await run(['--help'], help)).toBe(0);
        expect(help.out()).not.toMatch(/--force|-f,|--raw/);

        const out = join(cwd, 'out');
        await mkdir(out);
        await writeFile(join(out, 'note.txt'), 'taken');
        for (let n = 1; n < 1000; n++) await writeFile(join(out, `note (${n}).txt`), 'taken');
        const before = (await readdir(out)).sort();

        const io = testIo();
        const code = await run(['get', link, '-o', out], io);
        expect(code).not.toBe(0);
        expect(code).not.toBe(2);
        expect(io.err()).toContain('note.txt');
        expect(io.err()).toContain('no free name');
        expect((await readdir(out)).sort()).toEqual(before);
    });

    it('leaves the output location as it was when a write fails', async () => {
        const single = await putFiles([['note.txt', 'lost']]);
        const multi = await putFiles([
            ['a.txt', 'a'],
            ['b.txt', 'b'],
        ]);
        const out = join(cwd, 'out');
        await mkdir(out);
        await writeFile(join(out, 'other.txt'), 'x');
        const beforeOut = await readdir(out);
        const beforeCwd = await readdir(cwd);

        fsHooks.failWrites = true;
        expect(await run(['get', single, '-o', out], testIo())).not.toBe(0);
        expect(await readdir(out)).toEqual(beforeOut);
        fsHooks.failWrites = false;

        // The first item is written, the second fails: both the file and the
        // capsule's new directory are removed, and so is the file in cwd.
        for (const flags of [[], ['--here'], ['-o', out]]) {
            fsHooks.opens = 0;
            fsHooks.failOpenNumber = 2;
            const io = testIo();
            expect(await run(['get', multi, ...flags], io), flags.join(' ')).not.toBe(0);
            expect(fsHooks.opens).toBe(2);
            expect(await readdir(cwd)).toEqual(beforeCwd);
            expect(await readdir(out)).toEqual(beforeOut);
        }
    });

    it('writes a multi-item capsule into a new directory named after the date and id', async () => {
        const link = await putFiles([
            ['a.txt', 'a'],
            ['b.txt', 'b'],
        ]);
        const name = capsuleDir(link);
        await writeFile(join(cwd, name), 'a file');
        await symlink(join(cwd, 'elsewhere'), join(cwd, `${name} (1)`));
        const dir = join(cwd, `${name} (2)`);

        const io = testIo();
        expect(await run(['get', link], io), io.err()).toBe(0);
        expect(io.out()).toBe('');
        expect(io.err()).toContain(`created ${dir}\n`);
        expect(wrotePaths(io)).toEqual([join(dir, 'a.txt'), join(dir, 'b.txt')]);
        expect(await readFile(join(dir, 'b.txt'), 'utf8')).toBe('b');
        expect(await readFile(join(cwd, name), 'utf8')).toBe('a file');
        await expect(access(join(cwd, 'elsewhere'))).rejects.toThrow();

        const json = testIo();
        expect(await run(['get', link, '--json'], json), json.err()).toBe(0);
        const result = JSON.parse(json.out());
        const next = join(cwd, `${name} (3)`);
        expect(result.directory).toBe(next);
        expect(result.items.map((i: { path: string }) => i.path)).toEqual([
            join(next, 'a.txt'),
            join(next, 'b.txt'),
        ]);
    });

    it('writes a single item into a new owner-only directory, not into cwd', async () => {
        const link = await putFiles([['.capsulebin-probe', 'probe']]);
        const name = capsuleDir(link);
        const dir = join(cwd, name);

        const io = testIo();
        expect(await run(['get', link], io), io.err()).toBe(0);
        expect(io.out()).toBe('');
        expect(io.err()).toContain(`created ${dir}\n`);
        expect(wrotePaths(io)).toEqual([join(dir, '_capsulebin-probe')]);
        expect(await readdir(cwd)).toEqual([name]);
        expect(await readdir(dir)).toEqual(['_capsulebin-probe']);
        expect(await readFile(join(dir, '_capsulebin-probe'), 'utf8')).toBe('probe');
        if (posix) {
            expect(await mode(dir)).toBe(0o700);
            expect(await mode(join(dir, '_capsulebin-probe'))).toBe(0o600);
        }

        const json = testIo();
        expect(await run(['get', link, '--json'], json), json.err()).toBe(0);
        const result = JSON.parse(json.out());
        const next = join(cwd, `${name} (1)`);
        expect(result.directory).toBe(next);
        expect(result.items.map((i: { path: string }) => i.path)).toEqual([
            join(next, '_capsulebin-probe'),
        ]);
        expect((await readdir(cwd)).sort()).toEqual([name, `${name} (1)`]);
    });

    it('writes items directly into the current directory with --here, or into -o DIR', async () => {
        const link = await putFiles([
            ['a.txt', 'a'],
            ['b.txt', 'b'],
        ]);
        const single = await putFiles([['.capsulebin-probe', 'probe']]);
        await writeFile(join(cwd, 'a.txt'), 'kept');

        const here = testIo();
        expect(await run(['get', link, '--here'], here), here.err()).toBe(0);
        expect(here.err()).not.toContain('created ');
        expect(wrotePaths(here)).toEqual([join(cwd, 'a (1).txt'), join(cwd, 'b.txt')]);
        expect(await readFile(join(cwd, 'a.txt'), 'utf8')).toBe('kept');
        expect(await readFile(join(cwd, 'a (1).txt'), 'utf8')).toBe('a');

        const one = testIo();
        expect(await run(['get', single, '--here', '--json'], one), one.err()).toBe(0);
        const result = JSON.parse(one.out());
        expect(result.directory).toBe(cwd);
        expect(result.items[0].path).toBe(join(cwd, '_capsulebin-probe'));
        expect(await readFile(join(cwd, '_capsulebin-probe'), 'utf8')).toBe('probe');
        expect((await readdir(cwd)).sort()).toEqual([
            '_capsulebin-probe',
            'a (1).txt',
            'a.txt',
            'b.txt',
        ]);

        const out = testIo();
        expect(await run(['get', link, '-o', 'out'], out), out.err()).toBe(0);
        expect(wrotePaths(out)).toEqual([join(cwd, 'out', 'a.txt'), join(cwd, 'out', 'b.txt')]);
        expect(out.err()).not.toContain('created ');

        const before = server.requests.length;
        const both = testIo();
        expect(await run(['get', link, '--here', '-o', 'out'], both)).toBe(2);
        expect(both.err()).toContain('--here');
        expect(server.requests).toHaveLength(before);

        const help = testIo();
        expect(await run(['--help'], help)).toBe(0);
        const getSection = help.out().split('get options:')[1]!.split('delete options:')[0]!;
        const text = getSection.replace(/\s+/g, ' ');
        expect(text).toContain(
            'default: every capsule, one item included, goes to a new directory capsule-DATE-ID',
        );
        expect(text).toContain('--here Write the items directly into the current directory');
        expect(text).not.toContain('single item goes to the current directory');
    });

    it.each([
        ['note.txt', 'plain text\n'],
        ['main.rs', 'fn main() {}\n'],
        ['README.md', '# Title\n'],
        ['blob.bin', new Uint8Array([0, 1, 2, 0x1b, 0x9b, 255])],
    ])('writes a single item %s to a file, never to stdout', async (name, content) => {
        const link = await putFiles([[name, content]]);

        const io = testIo();
        expect(await run(['get', link], io), io.err()).toBe(0);
        expect(io.out()).toBe('');
        const dir = join(cwd, capsuleDir(link));
        expect(wrotePaths(io)).toEqual([join(dir, name)]);
        expect(new Uint8Array(await readFile(join(dir, name)))).toEqual(
            typeof content === 'string' ? encodeUtf8(content) : content,
        );

        const json = testIo();
        expect(await run(['get', link, '--json'], json), json.err()).toBe(0);
        const result = JSON.parse(json.out());
        const next = join(cwd, `${capsuleDir(link)} (1)`);
        expect(result.directory).toBe(next);
        expect(result.items[0].path).toBe(join(next, name));
    });

    it('never writes a name starting with a dot, also with --here or -o', async () => {
        const link = await putFiles([
            ['.bash_profile', 'echo owned'],
            ['.envrc', 'export X=1'],
        ]);
        const names = ['_bash_profile', '_envrc'];

        const intoNew = testIo();
        expect(await run(['get', link], intoNew), intoNew.err()).toBe(0);
        const dir = join(cwd, capsuleDir(link));
        expect((await readdir(dir)).sort()).toEqual(names);

        const out = join(cwd, 'out');
        const intoDir = testIo();
        expect(await run(['get', link, '-o', out], intoDir), intoDir.err()).toBe(0);
        expect((await readdir(out)).sort()).toEqual(names);
        expect(await readFile(join(out, '_bash_profile'), 'utf8')).toBe('echo owned');

        const home = join(cwd, 'home');
        await mkdir(home);
        const here = { ...testIo(), cwd: home };
        expect(await run(['get', link, '--here'], here), here.err()).toBe(0);
        expect((await readdir(home)).sort()).toEqual(names);
    });

    it('keeps suffixed names of long item names within the name limit', async () => {
        const long = `${'n'.repeat(251)}.txt`;
        expect(Buffer.byteLength(long)).toBe(255);
        const link = await putFiles([
            [long, 'first'],
            [long, 'second'],
        ]);
        const short = `${'n'.repeat(245)}.txt`;

        const io = testIo();
        expect(await run(['get', link, '--json'], io), io.err()).toBe(0);
        const paths = JSON.parse(io.out()).items.map((i: { path: string }) => i.path);
        const dir = join(cwd, capsuleDir(link));
        expect(paths).toEqual([join(dir, short), join(dir, `${'n'.repeat(245)} (1).txt`)]);
        expect(await readFile(paths[0], 'utf8')).toBe('first');
        expect(await readFile(paths[1], 'utf8')).toBe('second');

        // An existing entry at the shortened name in -o DIR: the next free name.
        const out = join(cwd, 'out');
        await mkdir(out);
        await writeFile(join(out, short), 'kept');
        for (let n = 1; n <= 2; n++)
            await writeFile(join(out, `${'n'.repeat(245)} (${n}).txt`), 'kept');
        const again = testIo();
        expect(await run(['get', link, '-o', out, '--json'], again), again.err()).toBe(0);
        const written = JSON.parse(again.out()).items.map((i: { path: string }) => i.path);
        expect(written).toEqual([
            join(out, `${'n'.repeat(245)} (3).txt`),
            join(out, `${'n'.repeat(245)} (4).txt`),
        ]);
        expect(await readFile(join(out, short), 'utf8')).toBe('kept');
    });

    // Linux: PATH_MAX is 4096. A path that fits only with a short name makes
    // the item's own name fail with ENAMETOOLONG; the item is still written.
    it.skipIf(process.platform !== 'linux')(
        'falls back to item-N when the file system refuses the name as too long',
        async () => {
            const name = `${'n'.repeat(200)}.txt`;
            const link = await putFiles([[name, 'content']]);
            let deep = join(cwd, 'deep');
            while (deep.length < 4096 - 120) deep = join(deep, 'd'.repeat(100));
            deep = deep.slice(0, 4096 - 120).replace(/\/+$/, '');

            const io = testIo();
            expect(await run(['get', link, '-o', deep, '--json'], io), io.err()).toBe(0);
            const path = JSON.parse(io.out()).items[0].path;
            expect(path).toBe(join(deep, 'item-1'));
            expect(await readFile(path, 'utf8')).toBe('content');
        },
    );

    // Acceptance (AC17): the capsule's directory is created non-recursively, so an
    // existing directory of that name is not reused.
    it('does not reuse an existing directory with the capsule directory name', async () => {
        const link = await putFiles([
            ['a.txt', 'a'],
            ['b.txt', 'b'],
        ]);
        const name = capsuleDir(link);
        await mkdir(join(cwd, name));
        await writeFile(join(cwd, name, 'mine.txt'), 'mine');

        const io = testIo();
        expect(await run(['get', link], io), io.err()).toBe(0);
        const dir = join(cwd, `${name} (1)`);
        expect(io.err()).toContain(`created ${dir}\n`);
        expect(wrotePaths(io)).toEqual([join(dir, 'a.txt'), join(dir, 'b.txt')]);
        expect(await readdir(join(cwd, name))).toEqual(['mine.txt']);
        expect(await readFile(join(cwd, name, 'mine.txt'), 'utf8')).toBe('mine');
    });

    // Acceptance (AC19): no content on stdout even when stdout is a terminal.
    it('writes a single item to a file when stdout is a terminal, also with -o', async () => {
        const link = await putFiles([['note.txt', 'on a tty \x1b[31m\n']]);
        for (const args of [[], ['-o', 'out']]) {
            const io = testIo();
            (io.stdout as { isTTY?: boolean }).isTTY = true;
            expect(await run(['get', link, ...args], io), io.err()).toBe(0);
            expect(io.out()).toBe('');
            const dir = join(cwd, args.length ? 'out' : capsuleDir(link));
            expect(wrotePaths(io)).toEqual([join(dir, 'note.txt')]);
            expect(await readFile(join(dir, 'note.txt'), 'utf8')).toBe('on a tty \x1b[31m\n');
        }
    });

    // Acceptance (AC4): a failed write of a single item removes the new
    // `capsule-…` directory it was written into.
    it('removes the new capsule directory when a single item write fails', async () => {
        const single = await putFiles([['note.txt', 'lost']]);
        const before = (await readdir(cwd)).sort();
        fsHooks.failWrites = true;
        const io = testIo();
        expect(await run(['get', single], io)).not.toBe(0);
        expect(io.out()).toBe('');
        expect((await readdir(cwd)).sort()).toEqual(before);
    });
});

describe('terminal output', () => {
    /**
     * Publishes a capsule with one plain-text item per entry of `contents`
     * (bytes need not be UTF-8), each named `options.name`.
     */
    async function publish(
        contents: Uint8Array | Uint8Array[],
        options: { name?: string; title?: string } = {},
    ): Promise<string> {
        const paste = new Paste({ title: options.title ?? null, burnAfterRetrieval: false });
        for (const bytes of Array.isArray(contents) ? contents : [contents]) {
            paste.addItem({
                id: generateUlid(),
                name: options.name ?? 'note.txt',
                size: bytes.length,
                mediaType: 'text/plain',
                presentation: 'plain',
                bytes,
            });
        }
        return (await paste.publish(new PasteApi(SERVER), SERVER)).shareUrl;
    }

    /** Answers every request with a problem of `status`, `title` and `code`. */
    function problems(status: number, title: string, code: string) {
        vi.stubGlobal(
            'fetch',
            async () =>
                new Response(JSON.stringify({ title, status, code }), {
                    status,
                    headers: { 'Content-Type': 'application/problem+json' },
                }),
        );
    }

    /** Every line of `stderr` is free of control characters but its trailing newline. */
    function expectEscapedLines(stderr: string) {
        expect(stderr.endsWith('\n')).toBe(true);
        for (const line of stderr.slice(0, -1).split('\n')) {
            expect(line).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/);
        }
    }

    const FORGED = 'bad \x1b[2J\ncapsulebin-cli: forged \u009b';
    const FORGED_SHOWN = 'bad \\x1b[2J\\x0acapsulebin-cli: forged \\u009b';

    it.each([
        { status: 400, exit: 6, what: 'title' },
        { status: 500, exit: 7, what: 'code' },
    ])('escapes a server problem $what on stderr ($status)', async ({ status, exit }) => {
        const link = `${SERVER}/p/${'A'.repeat(22)}#v1.${'A'.repeat(43)}`;
        problems(status, FORGED, FORGED);
        const io = testIo();
        expect(await run(['get', link], io)).toBe(exit);
        expect(io.err()).toContain(FORGED_SHOWN);
        expectEscapedLines(io.err());
        expect(io.err().match(/^capsulebin-cli:/gm)).toHaveLength(1);
    });

    it.each([
        { status: 404, exit: 3, title: FORGED, code: 'not_found' },
        { status: 409, exit: 6, title: 'Conflict', code: FORGED },
        { status: 503, exit: 7, title: FORGED, code: 'unavailable' },
        { status: 502, exit: 7, title: 'Bad gateway', code: FORGED },
    ])(
        'escapes a hostile title or code on its own on stderr ($status)',
        async ({ status, exit, title, code }) => {
            const link = `${SERVER}/p/${'A'.repeat(22)}#v1.${'A'.repeat(43)}`;
            problems(status, title, code);
            const io = testIo();
            expect(await run(['get', link], io)).toBe(exit);
            expectEscapedLines(io.err());
            expect(io.err().match(/^capsulebin-cli:/gm)).toHaveLength(1);
            expect(io.err()).not.toContain('\ncapsulebin-cli: forged');
        },
    );

    it('escapes the output directory and item names in wrote lines', async () => {
        const name = 'x\x1b[31m\n\u009b.txt';
        const shareUrl = await publish([encodeUtf8('one'), encodeUtf8('two')], { name });

        const io = testIo();
        expect(await run(['get', shareUrl, '-o', 'out\x1b\n\u009b'], io), io.err()).toBe(0);
        expect(io.err()).toContain('out\\x1b\\x0a\\u009b');
        expect(io.err().match(/^wrote /gm)).toHaveLength(2);
        expectEscapedLines(io.err());
    });

    it('escapes the delete link file in the lines that name it', async () => {
        const path = join(cwd, 'link\u001b[2J');
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io), io.err()).toBe(0);
        expect(io.err()).toBe(`delete link written to ${escapeDiagnostic(path)}\n`);
        expectEscapedLines(io.err());
        await access(path);

        vi.stubGlobal('fetch', async () => {
            throw new TypeError('connection refused');
        });
        const kept = join(cwd, 'kept\u001b[2J');
        const failed = testIo({ stdin: 'x' });
        expect(
            await run(['put', '-s', SERVER, '--retries', '0', '--delete-link-file', kept], failed),
        ).toBe(7);
        expect(failed.err()).toContain(escapeDiagnostic(kept));
        expectEscapedLines(failed.err());
        await access(kept);
    });

    it('escapes every control character for the last-resort message', () => {
        expect(escapeDiagnostic('a\x1b[2J\n\tb\x7f\u009bc ü')).toBe(
            'a\\x1b[2J\\x0a\\x09b\\x7f\\u009bc ü',
        );
    });

    /** `stdout` is one JSON line with DEL and C1 written as \u escapes. */
    function jsonOutput(io: TestIo): unknown {
        const text = io.out();
        expect(text.endsWith('}\n')).toBe(true);
        expect(text.slice(0, -1)).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/);
        return JSON.parse(text);
    }

    it('writes DEL and C1 as \\u escapes in every JSON output', async () => {
        const hostile = 'del \x7f c1 \u0080\u009b\u009f';
        const shown = 'del \\u007f c1 \\u0080\\u009b\\u009f';

        const put = testIo({ stdin: 'x' });
        const linkFile = `link ${hostile}`;
        expect(
            await run(['put', '-s', SERVER, '--json', '--delete-link-file', linkFile], put),
        ).toBe(0);
        expect(put.out()).toContain(shown);
        const created = jsonOutput(put) as { share_url: string; delete_link_file: string };
        expect(created.delete_link_file).toBe(join(cwd, linkFile));

        const link = await publish(encodeUtf8('x'), { name: hostile, title: hostile });
        const got = testIo();
        expect(await run(['get', '--json', '-o', 'out', link], got)).toBe(0);
        expect(got.out()).toContain(shown);
        const capsule = jsonOutput(got) as { title: string; items: { name: string }[] };
        expect(capsule.title).toBe(hostile);
        expect(capsule.items[0]!.name).toBe(hostile);

        const deleteLink = (await readFile(join(cwd, linkFile), 'utf8')).trim();
        const removed = testIo();
        expect(await run(['delete', '--json', deleteLink], removed)).toBe(0);
        expect(jsonOutput(removed)).toMatchObject({ deleted: true });

        problems(400, hostile, hostile);
        const failed = testIo();
        expect(await run(['get', '--json', link], failed)).toBe(6);
        expect(failed.out()).toContain(shown);
        expect(jsonOutput(failed)).toEqual({ error: { reason: hostile, message: hostile } });
    });
});

describe('acceptance edges of the hardened sources and delete link', () => {
    // AC1/AC2: the `--option=` form is as empty as `--option ""`.
    it('refuses an empty path given in the --option= form, without a fallback', async () => {
        await writeFile(join(cwd, 'pw'), 'hunter22\n');
        const created = await putText('x', '--password-file', 'pw');
        server.creationToken = 'cbt_from_env';
        const before = server.requests.length;
        const entries = (await readdir(cwd)).sort();
        const env = {
            CAPSULEBIN_TOKEN: 'cbt_from_env',
            CAPSULEBIN_PASSWORD: 'hunter22',
            CAPSULEBIN_SERVER: SERVER,
        };
        const cases: string[][] = [
            ['put', '-s', SERVER, '--password-file='],
            ['put', '-s', SERVER, '--token-file='],
            ['put', '-s', SERVER, '--delete-link-file='],
            ['get', '--password-file=', created.share_url],
            ['get', '--output=', created.share_url],
            ['get', '--link-file='],
            ['delete', '--link-file='],
        ];
        for (const argv of cases) {
            const io = testIo({ stdin: 'x', env, interactive: true });
            expect(await run(argv, io), argv.join(' ')).toBe(2);
            expect(io.err()).toContain('needs a path');
            expect(io.prompts).toEqual([]);
        }
        expect(server.requests).toHaveLength(before);
        expect((await readdir(cwd)).sort()).toEqual(entries);
    });

    // AC8: a link file whose directory is missing cannot be written: nothing is sent.
    it('sends nothing when the delete link file directory is missing', async () => {
        for (const extra of [[], ['--json']]) {
            const io = testIo({ stdin: 'x' });
            const code = await run(
                ['put', '-s', SERVER, ...extra, '--delete-link-file', join(cwd, 'no', 'link')],
                io,
            );
            expect(code).not.toBe(0);
            expect(io.err()).not.toContain('may exist');
            expect(io.err()).not.toContain('/manage/');
            expect(io.out()).not.toContain('/manage/');
        }
        expect(server.requests).toHaveLength(0);
        expect(server.pastes.size).toBe(0);
        expect(await readdir(cwd)).toEqual([]);
    });

    // AC12: the other first-line sources on an endless special file.
    it.skipIf(process.platform === 'win32')(
        'ends get --password-file and put --token-file on /dev/zero with exit 2',
        async () => {
            await writeFile(join(cwd, 'pw'), 'hunter22\n');
            const created = await putText('x', '--password-file', 'pw');
            const before = server.requests.length;

            const token = testIo({ stdin: 'x' });
            expect(
                await run(['put', '-s', SERVER, '--token-file', '/dev/zero'], token),
                token.err(),
            ).toBe(2);
            expect(token.err()).toContain('token from file /dev/zero');

            const get = testIo({ interactive: true });
            expect(
                await run(['get', '--password-file', '/dev/zero', created.share_url], get),
                get.err(),
            ).toBe(2);
            expect(get.err()).toContain('password from file /dev/zero');
            expect(get.prompts).toEqual([]);
            expect(
                server.requests.slice(before).filter((r) => r.url.endsWith('/retrieve')),
            ).toEqual([]);
            expect(server.pastes.has(created.id)).toBe(true);
        },
    );

    // AC9: a failure after the create request, with JSON and a link file: the
    // JSON error carries delete_link_file and no delete_url, the link appears nowhere.
    it('puts delete_link_file, not delete_url, into the JSON error after a rejected create', async () => {
        server.creationToken = 'cbt_secret';
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--json', '--delete-link-file', 'link'], io)).toBe(
            6,
        );
        const { error } = JSON.parse(io.out());
        expect(Object.keys(error)).toEqual(['reason', 'message', 'delete_link_file']);
        expect(error.delete_link_file).toBe(join(cwd, 'link'));
        for (const output of [io.out(), io.err()]) expect(output).not.toContain('#v1.');
        expect(await readFile(join(cwd, 'link'), 'utf8')).toMatch(/\/manage\/.+#v1\./);
    });
});
