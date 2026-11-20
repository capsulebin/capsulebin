/**
 * The API client follows no redirect: every request carries a capability or a
 * creation token, so a redirect must not carry it on to another server. Runs
 * against two real HTTP servers, because `fetch` stand-ins do not implement
 * redirect handling.
 */

import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { ApiError, PasteApi } from '../src/api';
import { encodeUtf8 } from '../src/helpers/bytes';
import { Paste } from '../src/paste';

const ID = 'A'.repeat(22);

let redirecting: Server;
let target: Server;
let redirectingBase: string;
let targetRequests: string[];
let status: 307 | 308;

function listen(server: Server): Promise<string> {
    return new Promise((resolve) => {
        server.listen(0, '127.0.0.1', () => {
            resolve(`http://127.0.0.1:${(server.address() as AddressInfo).port}`);
        });
    });
}

function close(server: Server): Promise<void> {
    return new Promise((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
    });
}

beforeAll(async () => {
    targetRequests = [];
    target = createServer((request, response) => {
        targetRequests.push(`${request.method} ${request.url}`);
        response.writeHead(200, { 'Content-Type': 'application/json' }).end('{}');
    });
    const targetBase = await listen(target);

    redirecting = createServer((request, response) => {
        request.resume();
        response.writeHead(status, { Location: `${targetBase}${request.url}` }).end();
    });
    redirectingBase = await listen(redirecting);
});

afterAll(async () => {
    await Promise.all([close(redirecting), close(target)]);
});

async function sealedPaste() {
    const paste = new Paste({ expiresIn: 60, burnAfterRetrieval: false });
    const bytes = encodeUtf8('x');
    paste.addItem({
        id: '01J00000000000000000000000',
        name: 'x.txt',
        size: bytes.length,
        mediaType: 'text/plain',
        presentation: 'plain',
        bytes,
    });
    return paste.seal();
}

const calls: [string, (api: PasteApi) => Promise<unknown>][] = [
    ['create', async (api) => api.create(await sealedPaste())],
    ['describe', (api) => api.getDescriptor(ID, { describeToken: new Uint8Array(32) })],
    ['retrieve', (api) => api.retrieve(ID, { describeToken: 'AA', revealToken: 'AA' })],
    ['delete', (api) => api.delete(ID, 'AA')],
];

describe('PasteApi redirects', () => {
    for (const code of [307, 308] as const) {
        for (const [name, call] of calls) {
            it(`refuses a ${code} on ${name} without contacting the redirect target`, async () => {
                status = code;
                const api = new PasteApi(redirectingBase, { creationToken: 'cbt_secret' });

                const error = await call(api).then(
                    () => undefined,
                    (err: unknown) => err,
                );

                expect(error).toBeInstanceOf(ApiError);
                expect(error).toMatchObject({ code: 'redirect_refused' });
                expect((error as ApiError).message).not.toContain('127.0.0.1');
                expect(targetRequests).toEqual([]);
            });
        }
    }
});
