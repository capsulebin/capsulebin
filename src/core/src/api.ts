/**
 * Opaque HTTP client for the CapsuleBin API.
 *
 * Every method maps HTTP details to typed results and errors. Secrets
 * (reveal tokens, delete secrets) are never logged. The URL fragment is never
 * transmitted to the server.
 */

import type { SealedPaste } from './paste';
import { fromBase64url, toBase64url } from './helpers/bytes';
import { PublicHeader } from './public_header';

// Response types use snake_case to match the server's JSON keys exactly.

export interface CreateResponse {
    id: string;
    expires_at: number | null;
    burn_after_retrieval: boolean;
}

export interface DescriptorResponse {
    id: string;
    format_version: number;
    public_header: PublicHeader;
    /** The exact bytes the header decoded from, i.e. the AAD used to encrypt
     *  the ciphertext. Kept alongside the parsed header rather than requiring
     *  callers to re-derive it via `public_header.toAad()`, which would only
     *  be correct because of (and would tie decryption to) the wire format's
     *  canonical-serialization guarantee. */
    public_header_aad: Uint8Array;
    expires_at: number | null;
    burn_after_retrieval: boolean;
    payload_size: number;
}

export interface RetrieveResponse {
    public_header: string; // base64url
    ciphertext: string; // base64url
}

/** {@link ApiError} code for a redirect the client did not follow. */
export const REDIRECT_REFUSED = 'redirect_refused';

export class ApiError extends Error {
    constructor(
        message: string,
        /** HTTP status; `0` when no response arrived (network failure). */
        public status: number,
        /** The problem+json `code`. Branch on this, never on the message. */
        public code: string | null,
        /** Seconds from a 429's `Retry-After` header, if the server sent one. */
        public retryAfter: number | null = null,
    ) {
        super(message);
        this.name = 'ApiError';
    }
}

/**
 * The describe response arrived, but its public header failed
 * {@link PublicHeader.parseFromJson}: malformed, or password parameters
 * outside what this client accepts. A malformed capsule, not a network
 * problem — `openPaste` reports it as `invalid_capsule`. An `ApiError` with
 * status `0`, so callers that only know `ApiError` keep treating it as before.
 */
export class MalformedHeaderError extends ApiError {
    constructor() {
        super('malformed public header', 0, null);
        this.name = 'MalformedHeaderError';
    }
}

export interface PasteApiOptions {
    /**
     * Creation token for servers in `creation.mode = "token"`. Sent as a
     * bearer on create only — never on descriptor, retrieve or delete calls.
     */
    creationToken?: string | undefined;
}

/** Client for one CapsuleBin server, bound to `baseUrl` once instead of
 *  threading it through every call. */
export class PasteApi {
    #baseUrl: string;
    #creationToken: string | undefined;

    constructor(baseUrl: string, options: PasteApiOptions = {}) {
        this.#baseUrl = baseUrl;
        this.#creationToken = options.creationToken || undefined;
    }

    /**
     * Creates a paste from a {@link SealedPaste}.
     *
     * The server stores the opaque payload and returns lifecycle metadata.
     * Only the fields the wire format needs are read —
     * `linkSecret`/`deleteSecret` never leave the caller. `sealed.id` is
     * freshly random; a collision is reported as an ApiError so the caller
     * can retry with a freshly sealed paste. On success `response.id` is
     * always `sealed.id` — the server never assigns a different one, it only
     * ever accepts the given id or rejects the whole request — which is why
     * {@link Paste.publish} builds its URLs from `sealed.id` without waiting
     * on this response.
     *
     * Safe to repeat: sending the same sealed paste again (e.g. after a
     * timeout whose outcome is unknown) returns the original result instead of
     * `id_conflict`, because the server treats an identical re-submission of
     * an id as a replay (ADR-011).
     */
    public async create(sealed: SealedPaste): Promise<CreateResponse> {
        const headers: Record<string, string> = { 'Content-Type': 'application/json' };
        if (this.#creationToken) {
            headers['Authorization'] = `Bearer ${this.#creationToken}`;
        }
        return this.#request<CreateResponse>(this.#url('/api/v1/pastes'), {
            method: 'POST',
            headers,
            body: JSON.stringify({
                id: sealed.id,
                format_version: sealed.formatVersion,
                public_header: sealed.publicHeader,
                ciphertext: sealed.ciphertext,
                reveal_verifier: sealed.revealVerifier,
                delete_verifier: sealed.deleteVerifier,
                describe_verifier: sealed.describeVerifier,
                expires_in: sealed.expiresIn,
                burn_after_retrieval: sealed.burnAfterRetrieval,
            }),
        });
    }

    /**
     * Fetches the public descriptor for a paste: its public header and
     * lifecycle metadata.
     *
     * Needs a capability - the describe token derived from the share link
     * (recipients) or the delete secret (the management page). Without one
     * that matches, the answer is the same as for an unknown id. The server
     * sends the public header as base64url; this decodes and parses it, so
     * callers get both a ready-to-use {@link PublicHeader} and the exact AAD
     * bytes, rather than having to repeat either step themselves.
     */
    public async getDescriptor(
        id: string,
        capability: { describeToken: Uint8Array } | { deleteSecret: string },
    ): Promise<DescriptorResponse> {
        const body =
            'describeToken' in capability
                ? { describe_token: toBase64url(capability.describeToken) }
                : { delete_secret: capability.deleteSecret };
        const wire = await this.#request<{
            id: string;
            format_version: number;
            public_header: string; // base64url
            expires_at: number | null;
            burn_after_retrieval: boolean;
            payload_size: number;
        }>(this.#url(`/api/v1/pastes/${encodeURIComponent(id)}/describe`), {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        });

        const publicHeaderAad = fromBase64url(wire.public_header);

        let publicHeader: PublicHeader;
        try {
            publicHeader = PublicHeader.parseFromJson(new TextDecoder().decode(publicHeaderAad));
        } catch {
            throw new MalformedHeaderError();
        }

        return {
            id: wire.id,
            format_version: wire.format_version,
            public_header: publicHeader,
            public_header_aad: publicHeaderAad,
            expires_at: wire.expires_at,
            burn_after_retrieval: wire.burn_after_retrieval,
            payload_size: wire.payload_size,
        };
    }

    /**
     * Retrieves the ciphertext for a paste.
     *
     * Both tokens from the link are sent in the body and must match their
     * stored verifiers. The describe token proves the link; the server counts
     * a wrong reveal token (a wrong password) against the paste only then. A
     * wrong token, an expired paste and an unknown id all produce the same
     * generic error so that an observer cannot distinguish them.
     */
    public async retrieve(
        id: string,
        tokens: {
            describeToken: string; // base64url
            revealToken: string; // base64url
        },
    ): Promise<RetrieveResponse> {
        return this.#request<RetrieveResponse>(
            this.#url(`/api/v1/pastes/${encodeURIComponent(id)}/retrieve`),
            {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    describe_token: tokens.describeToken,
                    reveal_token: tokens.revealToken,
                }),
            },
        );
    }

    /**
     * Deletes a paste.
     *
     * The delete secret is sent in the body. A wrong secret and an unknown
     * id both return 404 with the same generic error.
     */
    public async delete(
        id: string,
        deleteSecret: string, // base64url
    ): Promise<void> {
        return this.#request<void>(this.#url(`/api/v1/pastes/${encodeURIComponent(id)}`), {
            method: 'DELETE',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ delete_secret: deleteSecret }),
        });
    }

    /** Builds the full API URL from this client's base URL. */
    #url(path: string): string {
        const base = this.#baseUrl.replace(/\/+$/, '');
        return `${base}${path}`;
    }

    /**
     * Sends one API request. Redirects are never followed: every request
     * carries a capability or creation token in its body or headers, and the
     * server never redirects an API path, so a redirect comes from a proxy
     * mistake or someone else. It ends in an `ApiError` with code
     * `redirect_refused`; the `Location` is server-controlled text and is
     * not passed on.
     */
    async #request<T>(url: string, options: RequestInit): Promise<T> {
        let response: Response;
        try {
            response = await fetch(url, { ...options, redirect: 'manual' });
        } catch {
            throw new ApiError('network error', 0, null);
        }

        // Browsers hide a refused redirect behind an opaque response (status
        // 0); Node returns the 3xx response itself.
        if (
            response.type === 'opaqueredirect' ||
            (response.status >= 300 && response.status < 400)
        ) {
            response.body?.cancel().catch(() => undefined);
            throw new ApiError('redirect not followed', response.status, REDIRECT_REFUSED);
        }

        // 204 No Content: success with no body.
        if (response.status === 204) {
            return undefined as T;
        }

        let body: unknown;
        try {
            body = await response.json();
        } catch {
            throw new ApiError(`unexpected response: ${response.status}`, response.status, null);
        }

        if (!response.ok) {
            const problem =
                typeof body === 'object' && body !== null ? (body as Record<string, unknown>) : {};
            const retryAfter = Number.parseInt(response.headers.get('Retry-After') ?? '', 10);
            throw new ApiError(
                typeof problem['title'] === 'string' ? problem['title'] : 'request failed',
                response.status,
                typeof problem['code'] === 'string' ? problem['code'] : null,
                Number.isFinite(retryAfter) ? retryAfter : null,
            );
        }

        return body as T;
    }
}
