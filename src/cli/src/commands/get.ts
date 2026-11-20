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

import { mkdir, open } from 'node:fs/promises';
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
            io.stderr.write(`binary file written to ${path}\n`);
        } else {
            io.stdout.write(item.bytes);
        }
    } else {
        const dir = resolve(io.cwd, options.output ?? '.');
        const paths = await writeItems(capsule.items, dir, options.force);
        if (options.json) {
            io.stdout.write(
                JSON.stringify({
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
                }) + '\n',
            );
        } else {
            for (const path of paths) io.stderr.write(`wrote ${path}\n`);
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
    const cleaned = base.replace(/[\u0000-\u001f\u007f<>:"|?*]/g, '_').trim();
    if (!cleaned || /^\.+$/.test(cleaned)) return `item-${index + 1}`;
    return cleaned;
}

/**
 * Writes each item to `dir`. Existing files are never overwritten unless
 * `force`: a free name is chosen instead (`name (1).ext`), because the
 * content may not be retrievable a second time.
 */
async function writeItems(
    items: readonly CapsuleItem[],
    dir: string,
    force: boolean,
): Promise<string[]> {
    await mkdir(dir, { recursive: true });
    const paths: string[] = [];
    const used = new Set<string>();

    for (const [index, item] of items.entries()) {
        const name = safeFileName(item.name, index);
        for (let n = 0; ; n++) {
            const candidate = n === 0 ? name : withSuffix(name, n);
            if (used.has(candidate)) continue;
            const path = join(dir, candidate);
            try {
                const handle = await open(path, force && n === 0 ? 'w' : 'wx');
                try {
                    await handle.writeFile(item.bytes);
                } finally {
                    await handle.close();
                }
            } catch (err) {
                if ((err as NodeJS.ErrnoException).code === 'EEXIST') continue;
                throw err;
            }
            used.add(candidate);
            paths.push(path);
            break;
        }
    }
    return paths;
}

function withSuffix(name: string, n: number): string {
    const dot = name.lastIndexOf('.');
    return dot > 0 ? `${name.slice(0, dot)} (${n})${name.slice(dot)}` : `${name} (${n})`;
}
