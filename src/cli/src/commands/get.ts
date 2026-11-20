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
 * output never refuses because of an existing file (it picks a free name
 * unless `--force`), and a multi-item capsule without `-o` lands in the
 * current directory instead of being rejected.
 */

import { randomUUID } from 'node:crypto';
import { link, lstat, mkdir, open, rename, unlink } from 'node:fs/promises';
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
import { escapeContent, escapeDiagnostic, jsonLine } from '../terminal';
import { checkTransport } from '../transport';

export interface GetOptions {
    link: string;
    /** The server trusted with `CAPSULEBIN_PASSWORD` (`--server`/`CAPSULEBIN_SERVER`), if any. */
    anchor: string | undefined;
    /** Plain HTTP to a host other than loopback was opted into. */
    allowHttp: boolean;
    output: string | undefined;
    passwordFile: string | undefined;
    yes: boolean;
    force: boolean;
    /** Write a single item to a terminal unchanged instead of escaping its control characters. */
    raw: boolean;
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
    const toStdout = !options.output && !options.json && capsule.items.length === 1;

    if (toStdout) {
        const item = capsule.items[0]!;
        if (item.presentation === 'download' && io.stdout.isTTY) {
            // Already retrieved — write it to a file rather than lose it.
            const [path] = await writeItems(capsule.items, io.cwd, options.force);
            io.stderr.write(`binary file written to ${escapeDiagnostic(path!)}\n`);
        } else {
            io.stdout.write(
                io.stdout.isTTY && !options.raw ? escapeContent(item.bytes) : item.bytes,
            );
        }
    } else {
        const dir = resolve(io.cwd, options.output ?? '.');
        const paths = await writeItems(capsule.items, dir, options.force);
        if (options.json) {
            io.stdout.write(
                jsonLine({
                    title: capsule.title,
                    expires_at: outcome.expiresAt,
                    burn_after_retrieval: outcome.burnAfterRetrieval,
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
            for (const path of paths) io.stderr.write(`wrote ${escapeDiagnostic(path)}\n`);
        }
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

/**
 * Writes each item to `dir`, owner-only (`0600`). Existing entries are never
 * overwritten unless `force`: a free name is chosen instead (`name (1).ext`),
 * because the content may not be retrievable a second time.
 *
 * Each item is first written to a fresh, exclusively created temporary file
 * in `dir` and then put in place, so an existing entry (a symlink above all)
 * is never written through and a failed write leaves nothing behind. With
 * `force` the entry is replaced by `rename`; a directory under the name still
 * gets a free name, since it cannot be replaced by a file.
 */
async function writeItems(
    items: readonly CapsuleItem[],
    dir: string,
    force: boolean,
): Promise<string[]> {
    await mkdir(dir, { recursive: true, mode: 0o700 });
    const paths: string[] = [];
    const used = new Set<string>();

    for (const [index, item] of items.entries()) {
        const name = safeFileName(item.name, index);
        const temp = await writeTemp(dir, item.bytes);
        try {
            for (let n = 0; ; n++) {
                const candidate = n === 0 ? name : withSuffix(name, n);
                if (used.has(candidate)) continue;
                const path = join(dir, candidate);
                if (!(await place(temp, item.bytes, path, force && n === 0))) continue;
                used.add(candidate);
                paths.push(path);
                break;
            }
        } finally {
            // Gone after a rename; otherwise the placed copy is a hard link or a copy.
            await unlink(temp).catch(() => {});
        }
    }
    return paths;
}

/** Writes `bytes` to a new `0600` file in `dir` and returns its path. */
async function writeTemp(dir: string, bytes: Uint8Array): Promise<string> {
    const temp = join(dir, `.capsulebin-${randomUUID()}.tmp`);
    await writeNew(temp, bytes);
    return temp;
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

/** `link` errors of file systems without hard links (vfat, exFAT, many FUSE and network mounts). */
const NO_HARD_LINKS = new Set(['EPERM', 'ENOTSUP', 'EOPNOTSUPP', 'EXDEV', 'EMLINK', 'ENOSYS']);

/**
 * Puts the file `temp` (holding `bytes`) at `path`: replacing whatever entry
 * is there if `replace` (except a directory), else only if the name is free.
 * Where the file system has no hard links, a free name gets a fresh copy of
 * `bytes` instead. Returns false if the name is taken.
 */
async function place(
    temp: string,
    bytes: Uint8Array,
    path: string,
    replace: boolean,
): Promise<boolean> {
    try {
        await (replace ? rename(temp, path) : link(temp, path));
        return true;
    } catch (err) {
        const code = (err as NodeJS.ErrnoException).code;
        if (code === 'EEXIST') return false;
        if (replace && (await lstat(path).catch(() => undefined))?.isDirectory()) return false;
        if (!replace && code !== undefined && NO_HARD_LINKS.has(code)) {
            return await writeCopy(path, bytes);
        }
        throw err;
    }
}

/** Writes `bytes` to `path` only if the name is free. Returns false if it is taken. */
async function writeCopy(path: string, bytes: Uint8Array): Promise<boolean> {
    try {
        await writeNew(path, bytes);
        return true;
    } catch (err) {
        if ((err as NodeJS.ErrnoException).code === 'EEXIST') return false;
        throw err;
    }
}

function withSuffix(name: string, n: number): string {
    const dot = name.lastIndexOf('.');
    return dot > 0 ? `${name.slice(0, dot)} (${n})${name.slice(dot)}` : `${name} (${n})`;
}
