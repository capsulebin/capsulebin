/**
 * Share and management links.
 *
 *   share:   <base>/p/<id>#v1.<base64url link secret>
 *   manage:  <base>/manage/<id>#v1.<base64url delete secret>
 *
 * The fragment never reaches the server: browsers strip it from requests, and
 * every consumer of a parsed link sends only the id (and a derived capability)
 * over the wire. Parsing lives here, not in each client, so the web client and
 * the CLI reject exactly the same malformed links.
 */

import { fromBase64url } from './helpers/bytes';
import { FRAGMENT_PREFIX } from './public_header';

/** Length of a paste id: 16 random bytes, base64url without padding. */
const PASTE_ID_LENGTH = 22;
const PASTE_ID_PATTERN = /^[A-Za-z0-9_-]{22}$/;

/** Both the link secret and the delete secret are 32 random bytes. */
const SECRET_BYTES = 32;

export type LinkKind = 'share' | 'manage';

const PATH_SEGMENT: Record<LinkKind, string> = { share: 'p', manage: 'manage' };

export interface ParsedLink {
    kind: LinkKind;
    /** Everything before `/p/<id>` or `/manage/<id>` — the server's base URL. */
    base: string;
    id: string;
    /** The decoded 32-byte secret from the fragment. Never send this anywhere. */
    secret: Uint8Array<ArrayBuffer>;
}

export class LinkError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'LinkError';
    }
}

/** True if `id` has the shape of a paste id. Says nothing about existence. */
export function isPasteId(id: string): boolean {
    return id.length === PASTE_ID_LENGTH && PASTE_ID_PATTERN.test(id);
}

/**
 * Extracts a paste id from user input that may be a bare id or any CapsuleBin
 * link (share, management, with or without a trailing slash). The fragment —
 * where a key lives — is cut off before anything else looks at the text, so
 * nothing from it can come back. Returns `null` if no id is found.
 */
export function pasteIdFromInput(input: string): string | null {
    const withoutFragment = input.trim().split('#', 1)[0] ?? '';
    const candidate = withoutFragment.replace(/\/+$/, '').split('/').at(-1) ?? '';
    return isPasteId(candidate) ? candidate : null;
}

/**
 * Parses a share or management link. Throws {@link LinkError} with a
 * user-presentable message on any malformation; never contacts a server.
 */
export function parseLink(link: string, kind: LinkKind): ParsedLink {
    let url: URL;
    try {
        url = new URL(link);
    } catch {
        throw new LinkError('not a valid URL');
    }
    if (url.protocol !== 'https:' && url.protocol !== 'http:') {
        throw new LinkError('the link must be an http(s) URL');
    }

    const segments = url.pathname.split('/');
    const id = segments.at(-1) ?? '';
    const marker = segments.at(-2);
    if (marker !== PATH_SEGMENT[kind]) {
        throw new LinkError(
            kind === 'share'
                ? 'not a share link (expected …/p/<id>#v1.…)'
                : 'not a management link (expected …/manage/<id>#v1.…)',
        );
    }
    if (!isPasteId(id)) {
        throw new LinkError('invalid capsule identifier');
    }

    const secret = parseFragment(url.hash);
    const basePath = segments.slice(0, -2).join('/');
    return { kind, base: `${url.origin}${basePath}`, id, secret };
}

/**
 * Decodes the secret from a fragment (`#v1.<base64url>`, with or without the
 * leading `#`).
 */
export function parseFragment(hash: string): Uint8Array<ArrayBuffer> {
    const fragment = hash.startsWith('#') ? hash.slice(1) : hash;
    if (fragment.length === 0) {
        throw new LinkError('the link is incomplete — no key found');
    }
    if (!fragment.startsWith(FRAGMENT_PREFIX)) {
        throw new LinkError('unsupported link format');
    }

    let secret: Uint8Array<ArrayBuffer>;
    try {
        secret = fromBase64url(fragment.slice(FRAGMENT_PREFIX.length));
    } catch {
        throw new LinkError('the key could not be decoded');
    }
    if (secret.length !== SECRET_BYTES) {
        throw new LinkError('the key has the wrong length');
    }
    return secret;
}
