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
    unlink,
    writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    concatByteArrays,
    decodeUtf8,
    encodeUtf8,
    fromBase64url,
    generateUlid,
    Paste,
    PasteApi,
    toBase64url,
} from '@capsulebin/core';
import { run, parseDuration } from '../src/cli';
import { safeFileName } from '../src/commands/get';
import type { Io } from '../src/io';
import { buildItem } from '../src/items';
import { escapeDiagnostic } from '../src/terminal';
import { createFakeServer, type FakeServer } from '../../core/tests/fake-server';

// While set, every file write fails after the file was opened; `linkError`
// makes every hard link fail with that code (a file system without hard links).
// `failOpenNumber` makes only the write through the n-th opened file fail.
const fsHooks = vi.hoisted(() => ({
    failWrites: false,
    linkError: undefined as string | undefined,
    failOpenNumber: undefined as number | undefined,
    opens: 0,
}));
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
        link: async (...args: Parameters<typeof fs.link>) => {
            if (fsHooks.linkError !== undefined) {
                throw Object.assign(new Error('hard links not supported'), {
                    code: fsHooks.linkError,
                });
            }
            return fs.link(...args);
        },
    };
});

const SERVER = 'https://capsule.test';

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
    fsHooks.linkError = undefined;
    fsHooks.failOpenNumber = undefined;
    fsHooks.opens = 0;
});

interface TestIo extends Io {
    out(): string;
    outBytes(): Uint8Array;
    err(): string;
    prompts: string[];
}

function testIo(
    options: {
        stdin?: string | Uint8Array;
        env?: Record<string, string>;
        interactive?: boolean;
        answers?: string[];
        stdoutTTY?: boolean;
    } = {},
): TestIo {
    const out: Uint8Array[] = [];
    const err: string[] = [];
    const answers = [...(options.answers ?? [])];
    const prompts: string[] = [];
    const encoder = new TextEncoder();
    return {
        stdout: {
            write: (chunk) => out.push(typeof chunk === 'string' ? encoder.encode(chunk) : chunk),
            isTTY: options.stdoutTTY ?? false,
        },
        stderr: { write: (chunk) => err.push(chunk) },
        readStdin: async () =>
            typeof options.stdin === 'string'
                ? encoder.encode(options.stdin)
                : (options.stdin ?? new Uint8Array()),
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
        outBytes: () => new Uint8Array(Buffer.concat(out)),
        err: () => err.join(''),
        prompts,
    };
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
    it('round-trips stdin through a capsule and back to stdout', async () => {
        const created = await putText('hello from stdin\n');
        expect(created.share_url).toMatch(new RegExp(`^${SERVER}/p/[A-Za-z0-9_-]{22}#v1\\.`));
        expect(created.delete_url).toMatch(new RegExp(`^${SERVER}/manage/${created.id}#v1\\.`));

        const io = testIo();
        expect(await run(['get', created.share_url], io)).toBe(0);
        expect(io.out()).toBe('hello from stdin\n');
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
        expect(io.out()).toBe('hallo');

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

    it('writes a multi-item capsule to the current directory when no -o is given', async () => {
        await writeFile(join(cwd, 'a.txt'), 'a');
        await writeFile(join(cwd, 'b.txt'), 'b');
        const io = testIo();
        await run(['put', '-s', SERVER, 'a.txt', 'b.txt'], io);

        const getIo = testIo();
        expect(await run(['get', io.out().trim()], getIo)).toBe(0);
        // Existing files are kept; the retrieved ones get free names.
        expect((await readdir(cwd)).sort()).toEqual(['a (1).txt', 'a.txt', 'b (1).txt', 'b.txt']);
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
        expect(io.out()).toBe('from stdin');

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
        expect(io.out()).toBe('from a file');

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

describe('put --delete-link-file', () => {
    it('writes the management link to a new owner-only file instead of stderr', async () => {
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'private' });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io), io.err()).toBe(0);
        const deleteUrl = await readFile(path, 'utf8');
        expect(deleteUrl).toMatch(
            new RegExp(`^${SERVER}/manage/[A-Za-z0-9_-]{22}#v1\\.[^\\n]+\\n$`),
        );
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

    it('removes the file when the create fails', async () => {
        server.creationToken = 'cbt_secret';
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--delete-link-file', path], io)).toBe(6);
        await expect(access(path)).rejects.toThrow();
    });

    it.each([
        [
            'a redirect',
            async () =>
                new Response(null, { status: 307, headers: { Location: 'https://x.test/' } }),
        ],
        [
            'a network failure',
            async () => {
                throw new TypeError('connection refused');
            },
        ],
    ])('removes the file after %s', async (_, fake) => {
        vi.stubGlobal('fetch', fake);
        const path = join(cwd, 'manage-link');
        const io = testIo({ stdin: 'x' });
        expect(
            await run(['put', '-s', SERVER, '--retries', '0', '--delete-link-file', path], io),
        ).toBe(7);
        await expect(access(path)).rejects.toThrow();
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
        expect(ok.out()).toBe('secret note');
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
        expect(yes.out()).toBe('once');
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

    it('does not dump a binary single item onto a terminal', async () => {
        await writeFile(join(cwd, 'img.bin'), new Uint8Array([0, 159, 146, 150]));
        const put = testIo();
        await run(['put', '-s', SERVER, 'img.bin'], put);

        const io = testIo({ stdoutTTY: true });
        expect(await run(['get', put.out().trim()], io)).toBe(0);
        expect(io.outBytes()).toHaveLength(0);
        expect(io.err()).toContain('binary file written to');
        // The source file is still there, so the copy gets a free name.
        expect(await readFile(join(cwd, 'img (1).bin'))).toEqual(Buffer.from([0, 159, 146, 150]));
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
        expect(getIo.out()).toBe('x');
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
        expect(viaEnv.out()).toBe('over http');
        const viaFlag = testIo();
        expect(await run(['get', '--allow-http', created.share_url], viaFlag), viaFlag.err()).toBe(
            0,
        );
        expect(viaFlag.out()).toBe('over http');
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
        expect(io.out()).toBe('secret note');
    });

    it('uses the environment password for a link with a language segment', async () => {
        const shareUrl = (await protectedCapsule()).replace(`${SERVER}/`, `${SERVER}/de/`);
        const io = testIo({ env });
        expect(await run(['get', '-s', SERVER, shareUrl], io), io.err()).toBe(0);
        expect(io.out()).toBe('secret note');
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
        expect(right.out()).toBe('secret note');
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
            expect(io.out()).toBe('open');
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
        expect(ok.out()).toBe('spaced');
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
    ])('%j → %j', (name, expected) => {
        expect(safeFileName(name, 0)).toBe(expected);
    });

    it('leaves no C0, DEL or C1 character in a name', () => {
        // eslint-disable-next-line no-control-regex
        const controls = /[\u0000-\u001f\u007f-\u009f]/;
        expect(safeFileName('\u009b31m\u001b[2Jx.txt', 0)).not.toMatch(controls);
    });
});

describe('file output', () => {
    const posix = process.platform !== 'win32';
    const mode = async (path: string) => (await stat(path)).mode & 0o777;

    /** Puts files from `src/` (created with the given contents) and returns the share link. */
    async function putFiles(files: [string, string | Uint8Array][]): Promise<string> {
        const names: string[] = [];
        for (const [index, [name, content]] of files.entries()) {
            const dir = join(cwd, 'src', String(index));
            await mkdir(dir, { recursive: true });
            await writeFile(join(dir, name), content);
            names.push(join(dir, name));
        }
        const io = testIo();
        expect(await run(['put', '-s', SERVER, ...names], io), io.err()).toBe(0);
        return io.out().trim();
    }

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
            const binary = await putFiles([['img.bin', new Uint8Array([0, 159, 146, 150])]]);

            for (const force of [false, true]) {
                const flags = force ? ['--force'] : [];
                const sub = force ? 'forced' : 'plain';

                const intoCwd = testIo();
                expect(await run(['get', multi, ...flags], intoCwd), intoCwd.err()).toBe(0);
                const created = join(cwd, 'new', sub, 'out');
                const intoDir = testIo();
                expect(await run(['get', multi, '-o', created, ...flags], intoDir)).toBe(0);
                const tty = testIo({ stdoutTTY: true });
                expect(await run(['get', binary, ...flags], tty), tty.err()).toBe(0);

                const written = (intoCwd.err() + intoDir.err() + tty.err()).match(
                    /(?<=^(wrote|binary file written to) ).*$/gm,
                )!;
                expect(written).toHaveLength(5);
                for (const path of written) expect(await mode(path), path).toBe(0o600);
                expect(await mode(created)).toBe(0o700);
                expect(await mode(join(cwd, 'new', sub))).toBe(0o700);
            }
            expect(await mode(join(cwd, 'new'))).toBe(0o700);

            const existing = join(cwd, 'existing');
            await mkdir(existing);
            await chmod(existing, 0o755);
            expect(await run(['get', multi, '-o', existing], testIo())).toBe(0);
            expect(await mode(existing)).toBe(0o755);
            expect(await mode(join(existing, 'a.txt'))).toBe(0o600);
        });
    });

    it('never follows a symlink at the target name', async () => {
        const link = await putFiles([['note.txt', 'from the capsule']]);
        const out = join(cwd, 'out');
        await mkdir(out);
        const outside = join(cwd, 'outside.txt');
        await writeFile(outside, 'keep me');
        const dangling = join(cwd, 'nowhere.txt');

        for (const target of [outside, dangling]) {
            await symlink(target, join(out, 'note.txt'));

            const io = testIo();
            expect(await run(['get', link, '-o', out, '--json'], io), io.err()).toBe(0);
            const [item] = JSON.parse(io.out()).items;
            expect(item.path).toBe(join(out, 'note (1).txt'));
            expect(await readFile(item.path, 'utf8')).toBe('from the capsule');
            expect(await readlink(join(out, 'note.txt'))).toBe(target);

            const forced = testIo();
            expect(await run(['get', link, '-o', out, '-f', '--json'], forced)).toBe(0);
            expect(JSON.parse(forced.out()).items[0].path).toBe(join(out, 'note.txt'));
            const entry = await lstat(join(out, 'note.txt'));
            expect(entry.isFile()).toBe(true);
            if (posix) expect(entry.mode & 0o777).toBe(0o600);
            expect(await readFile(join(out, 'note.txt'), 'utf8')).toBe('from the capsule');

            expect(await readFile(outside, 'utf8')).toBe('keep me');
            await expect(access(dangling)).rejects.toThrow();
            expect((await readdir(out)).sort()).toEqual(['note (1).txt', 'note.txt']);
            for (const name of await readdir(out)) await unlink(join(out, name));
        }
    });

    it('replaces a file with --force, keeps duplicate names apart and skips a directory', async () => {
        const out = join(cwd, 'out');
        await mkdir(join(out, 'dir.txt'), { recursive: true });
        await writeFile(join(out, 'same.txt'), 'old');
        await chmod(join(out, 'same.txt'), 0o644);
        const link = await putFiles([
            ['same.txt', 'first'],
            ['same.txt', 'second'],
            ['dir.txt', 'not a directory'],
        ]);

        for (const flags of [['--force'], []]) {
            const io = testIo();
            expect(await run(['get', link, '-o', out, '--json', ...flags], io), io.err()).toBe(0);
            const paths = JSON.parse(io.out()).items.map((i: { path: string }) => i.path);
            if (flags.length) {
                expect(paths).toEqual([
                    join(out, 'same.txt'),
                    join(out, 'same (1).txt'),
                    join(out, 'dir (1).txt'),
                ]);
            }
            expect(await readFile(paths[0], 'utf8')).toBe('first');
            expect(await readFile(paths[1], 'utf8')).toBe('second');
            expect(await readFile(paths[2], 'utf8')).toBe('not a directory');
            if (posix) expect(await mode(join(out, 'same.txt'))).toBe(0o600);
            expect((await lstat(join(out, 'dir.txt'))).isDirectory()).toBe(true);
        }
        expect(await readFile(join(out, 'same.txt'), 'utf8')).toBe('first');
    });

    it('leaves no partial or temporary file when a write fails', async () => {
        const link = await putFiles([['note.txt', 'lost']]);
        const out = join(cwd, 'out');
        await mkdir(out);
        await writeFile(join(out, 'other.txt'), 'x');
        const before = await readdir(out);

        for (const flags of [[], ['--force']]) {
            fsHooks.failWrites = true;
            const io = testIo();
            expect(await run(['get', link, '-o', out, ...flags], io)).not.toBe(0);
            expect(await readdir(out)).toEqual(before);
        }
    });

    it('leaves nothing behind when the copy fails where hard links are unsupported', async () => {
        const link = await putFiles([['note.txt', 'lost']]);
        const out = join(cwd, 'out');
        await mkdir(out);
        await writeFile(join(out, 'other.txt'), 'x');
        const before = await readdir(out);

        fsHooks.linkError = 'EPERM';
        // Let the temporary file be written, then fail the write of the fallback copy (the second
        // file opened).
        fsHooks.opens = 0;
        fsHooks.failOpenNumber = 2;
        const io = testIo();
        expect(await run(['get', link, '-o', out], io)).not.toBe(0);
        expect(fsHooks.opens).toBeGreaterThanOrEqual(2);
        expect(await readdir(out)).toEqual(before);
    });

    it.each(['EPERM', 'ENOTSUP', 'EXDEV'])(
        'writes a copy where a hard link fails with %s, never through a symlink',
        async (code) => {
            const link = await putFiles([['note.txt', 'from the capsule']]);
            const out = join(cwd, 'out');
            await mkdir(out);
            const outside = join(cwd, 'outside.txt');
            await writeFile(outside, 'keep me');
            await symlink(outside, join(out, 'note.txt'));
            const dangling = join(cwd, 'nowhere.txt');
            await symlink(dangling, join(out, 'note (1).txt'));
            fsHooks.linkError = code;

            await withUmask(0o000, async () => {
                const io = testIo();
                expect(await run(['get', link, '-o', out, '--json'], io), io.err()).toBe(0);
                const [item] = JSON.parse(io.out()).items;
                expect(item.path).toBe(join(out, 'note (2).txt'));
                expect(await readFile(item.path, 'utf8')).toBe('from the capsule');
                if (posix) expect(await mode(item.path)).toBe(0o600);
            });

            expect(await readlink(join(out, 'note.txt'))).toBe(outside);
            expect(await readlink(join(out, 'note (1).txt'))).toBe(dangling);
            expect(await readFile(outside, 'utf8')).toBe('keep me');
            await expect(access(dangling)).rejects.toThrow();
            expect((await readdir(out)).sort()).toEqual([
                'note (1).txt',
                'note (2).txt',
                'note.txt',
            ]);
        },
    );
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

    // ESC, BEL, a lone CR, DEL, C1 CSI and an invalid byte, among text that stays.
    const HOSTILE = concatByteArrays(
        encodeUtf8('red \x1b[31m bell \x07 a\rb del \x7f csi \u009b bad '),
        new Uint8Array([0xff]),
        encodeUtf8(' tab\tcrlf\r\nGrüße ✓ 😀\n'),
    );
    const SHOWN =
        'red \\x1b[31m bell \\x07 a\\x0db del \\x7f csi \\u009b bad \\xff tab\tcrlf\r\nGrüße ✓ 😀\n';
    const C1 = /[\u0080-\u009f]/;

    it('shows control characters and invalid bytes of a single item as escapes on a TTY', async () => {
        const link = await publish(HOSTILE);
        const io = testIo({ stdoutTTY: true });
        expect(await run(['get', link], io), io.err()).toBe(0);

        expect(io.out()).toBe(SHOWN);
        const bytes = io.outBytes();
        for (const [i, byte] of bytes.entries()) {
            const crlf = byte === 0x0d && bytes[i + 1] === 0x0a;
            const allowed = byte === 0x09 || byte === 0x0a || crlf;
            expect(allowed || (byte >= 0x20 && byte !== 0x7f), `byte ${byte} at ${i}`).toBe(true);
        }
        expect(decodeUtf8(bytes)).not.toMatch(C1);
    });

    it('writes the item byte for byte to a pipe, and to a TTY with --raw', async () => {
        const link = await publish(HOSTILE);
        const piped = testIo();
        expect(await run(['get', link], piped)).toBe(0);
        expect(piped.outBytes()).toEqual(HOSTILE);

        const raw = testIo({ stdoutTTY: true });
        expect(await run(['get', '--raw', link], raw)).toBe(0);
        expect(raw.outBytes()).toEqual(HOSTILE);
    });

    it('accepts --raw only for get and lists it under the get options', async () => {
        const created = await putText('x');
        const put = testIo({ stdin: 'x' });
        expect(await run(['put', '-s', SERVER, '--raw'], put)).toBe(2);
        expect(put.err()).toContain('--raw does not apply to "put"');
        expect(await run(['delete', '--raw', created.delete_url], testIo())).toBe(2);
        expect(server.pastes.size).toBe(1);

        const help = testIo();
        expect(await run(['--help'], help)).toBe(0);
        const getSection = help.out().split('get options:')[1]!.split('delete options:')[0]!;
        expect(getSection).toContain('--raw');
    });

    /** Every line of `stderr` is free of control characters but its trailing newline. */
    function expectEscapedLines(stderr: string) {
        expect(stderr.endsWith('\n')).toBe(true);
        for (const line of stderr.slice(0, -1).split('\n')) {
            expect(line).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/);
        }
    }

    const FORGED = 'bad \x1b[2J\ncapsulebin: forged \u009b';
    const FORGED_SHOWN = 'bad \\x1b[2J\\x0acapsulebin: forged \\u009b';

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
        expect(io.err().match(/^capsulebin:/gm)).toHaveLength(1);
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
            expect(io.err().match(/^capsulebin:/gm)).toHaveLength(1);
            expect(io.err()).not.toContain('\ncapsulebin: forged');
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
