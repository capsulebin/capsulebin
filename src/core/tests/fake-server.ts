/**
 * An in-memory stand-in for the CapsuleBin HTTP API, installed as `fetch`.
 *
 * Implements just enough server semantics to drive the client flows end to
 * end in unit tests: verifiers are compared as SHA-256 hashes; unknown,
 * wrong-capability and already consumed burn pastes share one 404, exactly as
 * the real server answers them (410 only arises there when two retrievals of
 * one burn paste race — see `failNextBurnWithRace`); and an identical create
 * replay returns the original response (ADR-011).
 * It is not a conformance reference — the Rust server is.
 */

import { sha256 } from '../src/crypto';
import { concatByteArrays, encodeUtf8, fromBase64url, toBase64url } from '../src/helpers/bytes';

interface StoredPaste {
    body: string;
    publicHeader: string;
    ciphertext: string | null;
    revealVerifier: string;
    deleteVerifier: string;
    describeVerifier: string;
    expiresAt: number | null;
    burn: boolean;
    payloadSize: number;
}

export interface FakeServer {
    readonly pastes: Map<string, StoredPaste>;
    /** Every request seen, for asserting what did (not) go over the wire. */
    readonly requests: { method: string; url: string; headers: Headers; body: string }[];
    /** Creation token required on create, or `null` for public mode. */
    creationToken: string | null;
    /** Makes the next burn retrieval lose a simulated race: answers 410. */
    failNextBurnWithRace: boolean;
    /**
     * Answers every create success like a broken proxy in front of the server:
     * `204` with no body, or JSON `null`. The paste is still stored.
     */
    malformedCreate: 'no_content' | 'null' | null;
    fetch: typeof fetch;
}

function problem(status: number, code: string): Response {
    return new Response(JSON.stringify({ title: code, status, code }), {
        status,
        headers: { 'Content-Type': 'application/problem+json' },
    });
}

function json(status: number, value: unknown): Response {
    return new Response(JSON.stringify(value), {
        status,
        headers: { 'Content-Type': 'application/json' },
    });
}

async function hashB64(bytes: Uint8Array): Promise<string> {
    return toBase64url(await sha256(bytes));
}

export function createFakeServer(origin = 'http://capsule.test'): FakeServer {
    const server: FakeServer = {
        pastes: new Map(),
        requests: [],
        creationToken: null,
        failNextBurnWithRace: false,
        malformedCreate: null,
        fetch: async (input, init) => {
            const url = new URL(String(input), origin);
            const method = init?.method ?? 'GET';
            const headers = new Headers(init?.headers);
            const body = typeof init?.body === 'string' ? init.body : '';
            server.requests.push({ method, url: url.toString(), headers, body });
            return handle(server, method, url.pathname, headers, body);
        },
    };
    return server;
}

async function handle(
    server: FakeServer,
    method: string,
    path: string,
    headers: Headers,
    body: string,
): Promise<Response> {
    const match = /^\/api\/v1\/pastes(?:\/([A-Za-z0-9_-]{22})(?:\/(retrieve|describe))?)?$/.exec(
        path,
    );
    if (!match) return problem(404, 'not_found');
    const [, id, action] = match;
    const retrieve = action === 'retrieve';
    const describe = action === 'describe';

    if (!id && method === 'POST') {
        if (server.creationToken !== null) {
            if (headers.get('Authorization') !== `Bearer ${server.creationToken}`) {
                return problem(401, 'unauthorized');
            }
        }
        const request = JSON.parse(body) as Record<string, unknown>;
        const pasteId = String(request['id']);
        const existing = server.pastes.get(pasteId);
        if (existing) {
            return existing.body === body && existing.ciphertext !== null
                ? createSuccess(server, pasteId, existing)
                : problem(409, 'id_conflict');
        }
        const ciphertext = String(request['ciphertext']);
        const expiresIn = request['expires_in'];
        const stored: StoredPaste = {
            body,
            publicHeader: String(request['public_header']),
            ciphertext,
            revealVerifier: String(request['reveal_verifier']),
            deleteVerifier: String(request['delete_verifier']),
            describeVerifier: String(request['describe_verifier']),
            expiresAt:
                typeof expiresIn === 'number' ? Math.floor(Date.now() / 1000) + expiresIn : null,
            burn: request['burn_after_retrieval'] === true,
            payloadSize: fromBase64url(ciphertext).length,
        };
        server.pastes.set(pasteId, stored);
        return createSuccess(server, pasteId, stored);
    }

    const paste = id ? server.pastes.get(id) : undefined;

    if (id && describe && method === 'POST') {
        const request = JSON.parse(body) as Record<string, unknown>;
        let authorized = false;
        if (paste && typeof request['describe_token'] === 'string') {
            authorized =
                (await hashB64(fromBase64url(request['describe_token']))) ===
                paste.describeVerifier;
        } else if (paste && typeof request['delete_secret'] === 'string') {
            const verifier = await hashB64(
                concatByteArrays(
                    encodeUtf8('capsule/v1/delete'),
                    fromBase64url(request['delete_secret']),
                ),
            );
            authorized = verifier === paste.deleteVerifier;
        }
        if (!paste || !authorized || paste.ciphertext === null) {
            return problem(404, 'not_available');
        }
        return json(200, {
            id,
            format_version: 1,
            public_header: paste.publicHeader,
            expires_at: paste.expiresAt,
            burn_after_retrieval: paste.burn,
            payload_size: paste.payloadSize,
        });
    }

    if (id && retrieve && method === 'POST') {
        const request = JSON.parse(body) as Record<string, unknown>;
        const describeToken = String(request['describe_token']);
        const revealToken = String(request['reveal_token']);
        if (
            !paste ||
            (await hashB64(fromBase64url(describeToken))) !== paste.describeVerifier ||
            (await hashB64(fromBase64url(revealToken))) !== paste.revealVerifier ||
            paste.ciphertext === null
        ) {
            return problem(404, 'not_available');
        }
        if (paste.burn && server.failNextBurnWithRace) {
            server.failNextBurnWithRace = false;
            paste.ciphertext = null;
            return problem(410, 'already_retrieved');
        }
        const ciphertext = paste.ciphertext;
        if (paste.burn) paste.ciphertext = null;
        return json(200, { public_header: paste.publicHeader, ciphertext });
    }

    if (id && !action && method === 'DELETE') {
        const secret = String((JSON.parse(body) as Record<string, unknown>)['delete_secret']);
        const verifier = await hashB64(
            concatByteArrays(encodeUtf8('capsule/v1/delete'), fromBase64url(secret)),
        );
        if (!paste || verifier !== paste.deleteVerifier) return problem(404, 'not_available');
        server.pastes.delete(id);
        return new Response(null, { status: 204 });
    }

    return problem(405, 'method_not_allowed');
}

function createSuccess(server: FakeServer, id: string, paste: StoredPaste): Response {
    if (server.malformedCreate === 'no_content') return new Response(null, { status: 204 });
    if (server.malformedCreate === 'null') return json(201, null);
    return json(201, createResponse(id, paste));
}

function createResponse(id: string, paste: StoredPaste) {
    return { id, expires_at: paste.expiresAt, burn_after_retrieval: paste.burn };
}
