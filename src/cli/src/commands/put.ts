/**
 * `capsulebin put`: seal files or stdin into one capsule and upload it.
 *
 * Everything is encrypted here, before the request; the server receives only
 * the public header, ciphertext and verifiers (ADR-001). The capsule is sealed
 * exactly once and the same sealed request is re-sent on transient failures,
 * which the server answers like the original (ADR-011) — re-sealing would
 * create a second capsule.
 */

import { readFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import {
    ApiError,
    type CreateResponse,
    MAX_ITEMS,
    Paste,
    PasteApi,
    type SealedPaste,
} from '@capsulebin/core';
import { usage } from '../exit';
import type { Io } from '../io';
import { buildItem, type ItemInput, type Presentation } from '../items';
import { creationToken, newPassword } from '../secrets';

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
    const token = await creationToken(io, options.tokenFile);

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

    const api = new PasteApi(options.server, { creationToken: token });
    const created = await createWithRetry(io, api, sealed, options.retries);

    const shareUrl = paste.shareUrl(options.server);
    const deleteUrl = paste.deleteUrl(options.server);

    if (options.json) {
        io.stdout.write(
            JSON.stringify({
                id: created.id,
                share_url: shareUrl,
                delete_url: deleteUrl,
                expires_at: created.expires_at,
                burn_after_retrieval: created.burn_after_retrieval,
            }) + '\n',
        );
    } else {
        io.stdout.write(shareUrl + '\n');
        io.stderr.write(`delete link (keep it private): ${deleteUrl}\n`);
    }
}

async function readInputs(io: Io, options: PutOptions): Promise<ItemInput[]> {
    const sources = options.inputs.length > 0 ? options.inputs : ['-'];
    if (options.name && sources.length > 1) {
        throw usage('--name applies to a single input only');
    }

    const inputs: ItemInput[] = [];
    for (const source of sources) {
        if (source === '-') {
            const bytes = await io.readStdin();
            if (bytes.length === 0) throw usage('stdin is empty');
            inputs.push({ name: options.name ?? 'stdin', bytes });
            continue;
        }
        let bytes: Uint8Array;
        try {
            bytes = new Uint8Array(await readFile(resolve(io.cwd, source)));
        } catch (err) {
            const code = (err as NodeJS.ErrnoException).code;
            throw usage(
                code === 'ENOENT'
                    ? `${source}: no such file`
                    : code === 'EISDIR'
                      ? `${source}: is a directory`
                      : `${source}: cannot read file`,
            );
        }
        inputs.push({ name: options.name ?? basename(source), bytes });
    }
    return inputs;
}

/** Statuses worth a retry: no response, rate limited, or a server-side fault. */
function isTransient(err: unknown): err is ApiError {
    return err instanceof ApiError && (err.status === 0 || err.status === 429 || err.status >= 500);
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
                `capsulebin: ${err.status === 0 ? 'network error' : `HTTP ${err.status}`}, retrying in ${Math.round(delay / 1000)}s (${attempt + 1}/${retries})\n`,
            );
            await io.sleep(delay);
        }
    }
}
