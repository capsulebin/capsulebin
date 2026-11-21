/**
 * `capsulebin get <share link>`: retrieve, decrypt and write out a capsule.
 *
 * Runs the same `openPaste()` flow as the web view. Two decisions are made
 * *before* anything is retrieved, because a burn-after-retrieval capsule is
 * gone the moment the server hands it out:
 *
 * - whether to go ahead with a burn capsule (`--yes`, or a terminal prompt);
 * - the password, if the capsule needs one.
 *
 * After retrieval nothing may fail for a reason the user could have avoided:
 * an existing entry does not stop the output (it tries up to 1000 names,
 * `name`, `name (1)`, …, before failing with `no_free_name`). Without `-o`
 * or `--here` every capsule, a single item included, lands in a new private
 * directory, so a sender cannot place a file such as `.envrc` in the current
 * directory. Item content only ever goes to files, never to stdout.
 */

import { mkdir, open, rmdir, unlink } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import {
    type CapsuleItem,
    derivePasswordKeyFor,
    LinkError,
    openPaste,
    parseLink,
    PasteApi,
} from '@capsulebin/core';
import { alreadyRetrieved, CliError, ExitCode, fromApiError, toCliError, usage } from '../exit';
import type { Io } from '../io';
import { existingPassword } from '../secrets';
import { escapeDiagnostic, jsonLine } from '../terminal';
import { checkTransport } from '../transport';

export interface GetOptions {
    link: string;
    /** The server trusted with `CAPSULEBIN_PASSWORD` (`--server`/`CAPSULEBIN_SERVER`), if any. */
    anchor: string | undefined;
    /** Plain HTTP to a host other than loopback was opted into. */
    allowHttp: boolean;
    output: string | undefined;
    /** Write the items directly into the current directory, not into a new one. */
    here: boolean;
    passwordFile: string | undefined;
    yes: boolean;
    json: boolean;
}

export async function get(io: Io, options: GetOptions): Promise<void> {
    let link;
    try {
        link = parseLink(options.link, 'share');
    } catch (err) {
        if (err instanceof LinkError) throw usage(`invalid share link: ${err.message}`);
        throw err;
    }
    checkTransport(link.base, options.allowHttp, "the link's server");

    // A hook that stops the flow records why, so the reason survives
    // openPaste()'s coarse `aborted` outcome.
    let stopped: CliError | undefined;
    const recording =
        <T>(hook: () => Promise<T>) =>
        async (): Promise<T> => {
            try {
                return await hook();
            } catch (err) {
                stopped = toCliError(err);
                throw err;
            }
        };

    const outcome = await openPaste(new PasteApi(link.base), link.id, link.secret, {
        confirmBurn: recording(() => confirmBurn(io, options.yes)),
        passwordKey: (header) =>
            recording(async () =>
                derivePasswordKeyFor(
                    header,
                    await existingPassword(io, options.passwordFile, {
                        link: link.base,
                        anchor: options.anchor,
                    }),
                ),
            )(),
    });

    switch (outcome.kind) {
        case 'success':
            break;
        case 'burned':
            throw alreadyRetrieved();
        case 'api_error':
            throw fromApiError(outcome.error);
        case 'decrypt_failed':
            throw new CliError(
                'the capsule could not be decrypted — the link or password is wrong, or the content was modified',
                ExitCode.Corrupt,
                'decrypt_failed',
            );
        case 'invalid_capsule':
            throw new CliError(
                `the capsule is malformed: ${outcome.message}`,
                ExitCode.Corrupt,
                'invalid_capsule',
            );
        case 'password_required':
        case 'aborted':
            throw stopped ?? new CliError('aborted', ExitCode.NeedsInput, 'aborted');
    }

    const { capsule } = outcome;
    const output = await outputDirectory(io.cwd, link.id, options);
    let paths;
    try {
        paths = await writeItems(capsule.items, output.dir);
    } catch (err) {
        // Only an empty directory is removed: it holds nothing but what this run created.
        if (output.created) await rmdir(output.dir).catch(() => {});
        throw err;
    }

    if (options.json) {
        io.stdout.write(
            jsonLine({
                title: capsule.title,
                expires_at: outcome.expiresAt,
                burn_after_retrieval: outcome.burnAfterRetrieval,
                directory: output.dir,
                items: capsule.items.map((item, i) => ({
                    name: item.name,
                    size: item.size,
                    media_type: item.mediaType,
                    presentation: item.presentation,
                    language: item.language ?? null,
                    path: paths[i],
                })),
            }),
        );
    } else {
        if (output.created) io.stderr.write(`created ${escapeDiagnostic(output.dir)}\n`);
        for (const path of paths) io.stderr.write(`wrote ${escapeDiagnostic(path)}\n`);
    }

    if (outcome.burnAfterRetrieval) {
        io.stderr.write('this capsule has now been deleted from the server\n');
    }
}

async function confirmBurn(io: Io, yes: boolean): Promise<void> {
    if (yes) return;
    if (!io.interactive) {
        throw new CliError(
            'this capsule is deleted after retrieval; pass --yes to retrieve it non-interactively',
            ExitCode.NeedsInput,
            'confirmation_required',
        );
    }
    const answer = await io.prompt(
        'This capsule will be deleted from the server once retrieved. Continue? [y/N] ',
        false,
    );
    if (!/^y(es)?$/i.test(answer.trim())) {
        throw new CliError('not retrieved', ExitCode.NeedsInput, 'declined');
    }
}

/**
 * Reduces an item name from the (untrusted) manifest to a plain file name:
 * no directories, no control characters, no dot-only names. The manifest
 * name is display metadata, never a path (capsule-file-format.md).
 */
export function safeFileName(name: string, index: number): string {
    const base = name.split(/[/\\]/).at(-1) ?? '';
    // eslint-disable-next-line no-control-regex
    const cleaned = base.replace(/[\u0000-\u001f\u007f-\u009f<>:"|?*]/g, '_').trim();
    if (!cleaned || /^\.+$/.test(cleaned)) return `item-${index + 1}`;
    return cleaned;
}

/** Attempts at a free name (`name`, `name (1)`, …) before giving up. */
const MAX_NAME_ATTEMPTS = 1000;

/**
 * The directory the items go to: `-o DIR` (created `0700` with its parents
 * if missing), the current directory with `--here`, else a new `0700`
 * directory `capsule-<local date>-<id prefix>` in the current directory, also
 * for a single item. `created` is set only for that new directory.
 */
async function outputDirectory(
    cwd: string,
    id: string,
    options: Pick<GetOptions, 'output' | 'here'>,
): Promise<{ dir: string; created: boolean }> {
    if (options.output !== undefined) {
        const dir = resolve(cwd, options.output);
        await mkdir(dir, { recursive: true, mode: 0o700 });
        return { dir, created: false };
    }
    if (options.here) return { dir: cwd, created: false };

    const name = `capsule-${localDate(new Date())}-${id.slice(0, 8)}`;
    const dir = await createUnderFreeName(
        cwd,
        name,
        (n) => `${name} (${n})`,
        (path) => mkdir(path, { mode: 0o700 }).then(() => {}),
        `the capsule's directory ${name}`,
    );
    return { dir, created: true };
}

/** `YYYY-MM-DD` in local time. */
function localDate(date: Date): string {
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/**
 * Writes each item to a new owner-only (`0600`) file in `dir`. An existing
 * entry of any kind (file, symlink – dangling or not –, directory) is never
 * opened, followed or replaced: the item gets the next free name
 * (`name (1).ext`), because the content may not be retrievable a second time.
 * If a write fails, the files written so far are removed again.
 */
async function writeItems(items: readonly CapsuleItem[], dir: string): Promise<string[]> {
    const paths: string[] = [];
    try {
        for (const [index, item] of items.entries()) {
            const name = safeFileName(item.name, index);
            paths.push(
                await createUnderFreeName(
                    dir,
                    name,
                    (n) => withSuffix(name, n),
                    (path) => writeNew(path, item.bytes),
                    `item ${index + 1} (${name})`,
                ),
            );
        }
    } catch (err) {
        for (const path of paths) await unlink(path).catch(() => {});
        throw err;
    }
    return paths;
}

/**
 * Creates an entry in `dir` with `create`, which must fail with `EEXIST` when
 * the name is taken, under `name` or else the first free `suffixed(n)`.
 * Returns its path; fails naming `what` when no name is free.
 */
async function createUnderFreeName(
    dir: string,
    name: string,
    suffixed: (n: number) => string,
    create: (path: string) => Promise<void>,
    what: string,
): Promise<string> {
    for (let n = 0; n < MAX_NAME_ATTEMPTS; n++) {
        const path = join(dir, n === 0 ? name : suffixed(n));
        try {
            await create(path);
            return path;
        } catch (err) {
            if ((err as NodeJS.ErrnoException).code !== 'EEXIST') throw err;
        }
    }
    throw new CliError(
        `no free name for ${what} in ${dir} after ${MAX_NAME_ATTEMPTS} attempts`,
        ExitCode.Failure,
        'no_free_name',
    );
}

/**
 * Writes `bytes` to `path`, which must not exist yet (no entry of any kind,
 * not even a dangling symlink), as a `0600` file; removes it again if the
 * write fails.
 */
async function writeNew(path: string, bytes: Uint8Array): Promise<void> {
    const handle = await open(path, 'wx', 0o600);
    try {
        try {
            await handle.writeFile(bytes);
        } finally {
            await handle.close();
        }
    } catch (err) {
        await unlink(path).catch(() => {});
        throw err;
    }
}

function withSuffix(name: string, n: number): string {
    const dot = name.lastIndexOf('.');
    return dot > 0 ? `${name.slice(0, dot)} (${n})${name.slice(dot)}` : `${name} (${n})`;
}
