/**
 * `PasteApi.create` resolves only with the answer for the paste it sent: a
 * success without a usable body (a proxy's `204`, JSON `null`, another id, a
 * wrong field type) is an `INVALID_RESPONSE` error, so callers treat it as an
 * unclear outcome instead of failing later on a missing field.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import * as core from '../src/index';
import { ApiError, INVALID_RESPONSE, PasteApi } from '../src/api';
import { createFakeServer } from './fake-server';
import { sealedPaste } from './limits-support';

const ORIGIN = 'http://capsule.test';

afterEach(() => {
    vi.unstubAllGlobals();
});

/** Answers every request with `status` and `body` (JSON text, or `null` for none). */
function answer(status: number, body: string | null) {
    vi.stubGlobal(
        'fetch',
        async () =>
            new Response(body, {
                status,
                headers: body === null ? {} : { 'Content-Type': 'application/json' },
            }),
    );
}

describe('create response check', () => {
    it('exports the code from the package', () => {
        expect(core.INVALID_RESPONSE).toBe('invalid_response');
    });

    const valid = (id: string) => ({ id, expires_at: 1_900_000_000, burn_after_retrieval: false });
    const malformed: [string, number, (id: string) => string | null][] = [
        ['204 without a body', 204, () => null],
        ['JSON null', 201, () => 'null'],
        ['a string', 201, () => '"ok"'],
        ['a number', 201, () => '1'],
        ['an array', 201, (id) => JSON.stringify([valid(id)])],
        ['a missing id', 201, (id) => JSON.stringify({ ...valid(id), id: undefined })],
        ['a different id', 201, () => JSON.stringify(valid('A'.repeat(22)))],
        ['an id of another type', 201, () => JSON.stringify({ ...valid(''), id: 1 })],
        ['a missing burn flag', 201, (id) => JSON.stringify({ id, expires_at: null })],
        [
            'a burn flag that is no boolean',
            201,
            (id) => JSON.stringify({ ...valid(id), burn_after_retrieval: 'false' }),
        ],
        ['a missing expiry', 201, (id) => JSON.stringify({ id, burn_after_retrieval: true })],
        [
            'an expiry that is no number',
            201,
            (id) => JSON.stringify({ ...valid(id), expires_at: '1900000000' }),
        ],
    ];

    for (const [name, status, body] of malformed) {
        it(`rejects ${name}`, async () => {
            const sealed = await sealedPaste();
            answer(status, body(sealed.id));
            const error = await new PasteApi(ORIGIN).create(sealed).then(
                () => undefined,
                (err: unknown) => err,
            );
            expect(error).toBeInstanceOf(ApiError);
            expect(error).toMatchObject({ status: 0, code: INVALID_RESPONSE });
        });
    }

    it('resolves a valid answer, with and without an expiry', async () => {
        const sealed = await sealedPaste();
        for (const expires_at of [1_900_000_000, null]) {
            const response = { ...valid(sealed.id), expires_at };
            answer(201, JSON.stringify(response));
            await expect(new PasteApi(ORIGIN).create(sealed)).resolves.toEqual(response);
        }
    });

    it('resolves a create and its replay against the fake server', async () => {
        const server = createFakeServer(ORIGIN);
        vi.stubGlobal('fetch', server.fetch);
        const sealed = await sealedPaste();
        const api = new PasteApi(ORIGIN);

        const first = await api.create(sealed);
        const replay = await api.create(sealed);

        expect(first.id).toBe(sealed.id);
        expect(replay).toEqual(first);
        expect(server.requests).toHaveLength(2);
    });

    for (const variant of ['no_content', 'null'] as const) {
        it(`rejects the fake server's malformed create (${variant})`, async () => {
            const server = createFakeServer(ORIGIN);
            server.malformedCreate = variant;
            vi.stubGlobal('fetch', server.fetch);
            const sealed = await sealedPaste();

            await expect(new PasteApi(ORIGIN).create(sealed)).rejects.toMatchObject({
                status: 0,
                code: INVALID_RESPONSE,
            });
            expect(server.pastes.has(sealed.id)).toBe(true);
        });
    }
});
