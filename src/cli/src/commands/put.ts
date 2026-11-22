/**
 * `capsulebin-cli put`: seal files or stdin into one capsule and upload it.
 *
 * Everything is encrypted here, before the request; the server receives only
 * the public header, ciphertext and verifiers (ADR-001). The capsule is sealed
 * exactly once and the same sealed request is re-sent on transient failures,
 * which the server answers like the original (ADR-011) — re-sealing would
 * create a second capsule.
 */

import { createReadStream } from 'node:fs';
import { type FileHandle, open, stat, unlink } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import {
    ApiError,
    type CreateResponse,
    MAX_CAPSULE_BYTES,
    MAX_ITEMS,
    Paste,
    PasteApi,
    REDIRECT_REFUSED,
    RESPONSE_TOO_LARGE,
    type SealedPaste,
    TIMEOUT,
} from '@capsulebin/core';
import { type CliError, usage, withDeleteLink } from '../exit';
import { type Io, readBounded } from '../io';
import { buildItem, type ItemInput, type Presentation } from '../items';
import { PROGRAM } from '../program';
import { creationToken, newPassword } from '../secrets';
import { escapeDiagnostic, jsonLine } from '../terminal';

export interface PutOptions {
    inputs: string[];
    server: string;
    expiresIn: number | null | undefined;
    burn: boolean;
    password: boolean;
    passwordFile: string | undefined;
    tokenFile: string | undefined;
    as: Presentation | undefined;
    lang: string | undefined;
    name: string | undefined;
    title: string | undefined;
    retries: number;
    /** Write the management link to this new file instead of stderr/JSON. */
    deleteLinkFile: string | undefined;
    json: boolean;
}

export async function put(io: Io, options: PutOptions): Promise<void> {
    if (options.inputs.length > MAX_ITEMS) {
        throw usage(`a capsule holds at most ${MAX_ITEMS} items`);
    }
    if (options.inputs.filter((input) => input === '-').length > 1) {
        throw usage('stdin (-) can only be given once');
    }

    const inputs = await readInputs(io, options);
    const password = await newPassword(io, options);
    const token = await creationToken(io, options.tokenFile, options.server);

    // `expiresIn: undefined` omits `expires_in`, so the server applies its default.
    const paste = new Paste({
        title: options.title ?? null,
        password,
        expiresIn: options.expiresIn,
        burnAfterRetrieval: options.burn,
    });
    for (const input of inputs) {
        paste.addItem(buildItem(input, { as: options.as, lang: options.lang }));
    }

    const sealed = await paste.seal();
    const shareUrl = paste.shareUrl(options.server);
    const deleteUrl = paste.deleteUrl(options.server);

    const api = new PasteApi(options.server, { creationToken: token });
    const linkFile =
        options.deleteLinkFile === undefined ? undefined : resolve(io.cwd, options.deleteLinkFile);
    if (linkFile !== undefined) await writeLinkFile(linkFile, deleteUrl);
    let created: CreateResponse;
    try {
        created = await createWithRetry(io, api, sealed, options.retries);
    } catch (err) {
        // The capsule may exist whatever the failure says (a lost response, a
        // proxy's answer): from here on the delete link is never dropped.
        throw withDeleteLink(err, linkFile === undefined ? { url: deleteUrl } : { file: linkFile });
    }

    if (options.json) {
        io.stdout.write(
            jsonLine({
                id: created.id,
                share_url: shareUrl,
                ...(linkFile === undefined
                    ? { delete_url: deleteUrl }
                    : { delete_link_file: linkFile }),
                expires_at: created.expires_at,
                burn_after_retrieval: created.burn_after_retrieval,
            }),
        );
    } else {
        io.stdout.write(shareUrl + '\n');
        io.stderr.write(
            linkFile === undefined
                ? `delete link (keep it private): ${deleteUrl}\n`
                : `delete link written to ${escapeDiagnostic(linkFile)}\n`,
        );
    }
}

/**
 * Writes the management link to a new owner-only file. The file is created
 * exclusively (an existing file or symlink is refused) and written before the
 * upload, so no capsule exists whose link could not be stored. If the write
 * fails, the file is removed again: no request went out yet. After that it is
 * never removed, whatever becomes of the upload.
 */
async function writeLinkFile(path: string, deleteUrl: string): Promise<void> {
    let file: FileHandle;
    try {
        file = await open(path, 'wx', 0o600);
    } catch (err) {
        const code = (err as NodeJS.ErrnoException).code;
        throw usage(
            code === 'EEXIST'
                ? `--delete-link-file ${path} already exists`
                : `cannot create --delete-link-file ${path}`,
        );
    }
    try {
        await file.writeFile(deleteUrl + '\n');
        await file.close();
    } catch (err) {
        await file.close().catch(() => {});
        await unlink(path).catch(() => {});
        throw err;
    }
}

/**
 * Reads the inputs within one budget of `MAX_CAPSULE_BYTES` over all of them,
 * so a large input ends in a usage error instead of exhausting memory. The
 * sizes of regular files are checked before anything is read; files that
 * grow, special files reporting size 0 and stdin are cut off at the first
 * byte beyond the budget.
 */
async function readInputs(io: Io, options: PutOptions): Promise<ItemInput[]> {
    const sources = options.inputs.length > 0 ? options.inputs : ['-'];
    if (options.name && sources.length > 1) {
        throw usage('--name applies to a single input only');
    }

    let declared = 0;
    for (const source of sources) {
        if (source === '-') continue;
        let info;
        try {
            info = await stat(resolve(io.cwd, source));
        } catch (err) {
            throw fileError(source, err);
        }
        if (info.isDirectory()) throw usage(`${source}: is a directory`);
        if (info.isFile()) declared += info.size;
    }
    if (declared > MAX_CAPSULE_BYTES) throw inputTooLarge();

    const inputs: ItemInput[] = [];
    let remaining = MAX_CAPSULE_BYTES;
    for (const source of sources) {
        let bytes: Uint8Array;
        if (source === '-') {
            bytes = await io.readStdin(remaining);
            if (bytes.length === 0) throw usage('stdin is empty');
        } else {
            try {
                bytes = await readBounded(createReadStream(resolve(io.cwd, source)), remaining);
            } catch (err) {
                throw fileError(source, err);
            }
        }
        if (bytes.length > remaining) throw inputTooLarge();
        remaining -= bytes.length;
        inputs.push({ name: options.name ?? (source === '-' ? 'stdin' : basename(source)), bytes });
    }
    return inputs;
}

function inputTooLarge(): CliError {
    return usage(`the input exceeds the capsule limit of ${MAX_CAPSULE_BYTES} bytes`);
}

function fileError(source: string, err: unknown): CliError {
    const code = (err as NodeJS.ErrnoException).code;
    return usage(
        code === 'ENOENT'
            ? `${source}: no such file`
            : code === 'EISDIR'
              ? `${source}: is a directory`
              : `${source}: cannot read file`,
    );
}

/**
 * Statuses worth a retry: no response (a timeout included), rate limited, or
 * a server-side fault. Never a refused redirect, whatever its status, nor an
 * oversized response: the next attempt would get the same answer.
 */
function isTransient(err: unknown): err is ApiError {
    return (
        err instanceof ApiError &&
        err.code !== REDIRECT_REFUSED &&
        err.code !== RESPONSE_TOO_LARGE &&
        (err.status === 0 || err.status === 429 || err.status >= 500)
    );
}

const MAX_RETRY_DELAY_MS = 60_000;

async function createWithRetry(
    io: Io,
    api: PasteApi,
    sealed: SealedPaste,
    retries: number,
): Promise<CreateResponse> {
    for (let attempt = 0; ; attempt++) {
        try {
            return await api.create(sealed);
        } catch (err) {
            if (!isTransient(err) || attempt >= retries) throw err;
            const delay = Math.min(
                err.retryAfter !== null ? err.retryAfter * 1000 : 1000 * 2 ** attempt,
                MAX_RETRY_DELAY_MS,
            );
            io.stderr.write(
                `${PROGRAM}: ${err.code === TIMEOUT ? 'timeout' : err.status === 0 ? 'network error' : `HTTP ${err.status}`}, retrying in ${Math.round(delay / 1000)}s (${attempt + 1}/${retries})\n`,
            );
            await io.sleep(delay);
        }
    }
}
