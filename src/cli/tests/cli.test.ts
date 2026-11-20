/**
 * The CLI end to end against the in-memory API from @capsulebin/core's tests:
 * argument handling, put → get → delete round trips, the exit codes scripts
 * rely on, and the rules that protect secrets and burn capsules.
 */

import { mkdtemp, readdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { run, parseDuration } from '../src/cli';
import { safeFileName } from '../src/commands/get';
import type { Io } from '../src/io';
import { buildItem } from '../src/items';
import { createFakeServer, type FakeServer } from '../../core/tests/fake-server';

const SERVER = 'http://capsule.test';

let server: FakeServer;
let cwd: string;

beforeEach(async () => {
    server = createFakeServer(SERVER);
    vi.stubGlobal('fetch', server.fetch);
    cwd = await mkdtemp(join(tmpdir(), 'capsulebin-cli-'));
});

afterEach(() => {
    vi.unstubAllGlobals();
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
            await run(['get', shareUrl], testIo({ env: { CAPSULEBIN_PASSWORD: 'wrong' } })),
        ).toBe(3);
        const ok = testIo({ env: { CAPSULEBIN_PASSWORD: 'hunter22' } });
        expect(await run(['get', shareUrl], ok)).toBe(0);
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
    ])('%j → %j', (name, expected) => {
        expect(safeFileName(name, 0)).toBe(expected);
    });
});
