/**
 * The creation token on the create page (ADR-018): the instance's creation mode
 * from the page, the invitation link `#token=<token>`, and the token the browser
 * holds - remembered in `localStorage` or for this page only.
 *
 * The token goes nowhere but into `PasteApi`'s `creationToken`, which sends it as
 * the bearer of the create request only.
 */

/** Where a remembered token is kept. One origin per instance, so per instance. */
export const CREATION_TOKEN_KEY = 'capsulebin.creation_token';

/** Longest token accepted. Generated tokens are 43 characters. */
export const MAX_TOKEN_CHARS = 1024;

/** The fragment of an invitation link, followed by the token as it is. */
const INVITATION_PREFIX = '#token=';

/** `creation.mode` of the instance, as the server put it into the create page. */
export type CreationMode = 'public' | 'token' | 'disabled';

/**
 * The mode from `<meta name="capsulebin-creation">`. A missing or unknown value is
 * `public`, the page as it was before the meta existed: the server still decides.
 */
export function creationMode(): CreationMode {
    const value = document
        .querySelector<HTMLMetaElement>('meta[name="capsulebin-creation"]')
        ?.getAttribute('content');
    return value === 'token' || value === 'disabled' ? value : 'public';
}

/**
 * The token in `raw` after trimming, or `null` if it cannot be one: 1 to
 * {@link MAX_TOKEN_CHARS} characters, all visible ASCII (U+0021-U+007E). The
 * server reads the bearer as such a header value; anything else could never match.
 */
export function validToken(raw: string): string | null {
    const token = raw.trim();
    if (token.length < 1 || token.length > MAX_TOKEN_CHARS) return null;
    return /^[\x21-\x7e]+$/.test(token) ? token : null;
}

/** What a fragment says about an invitation. */
export type Invitation = { kind: 'none' } | { kind: 'token'; token: string } | { kind: 'invalid' };

/**
 * Reads an invitation link's fragment (`location.hash`). The value is taken as it
 * is, without percent-decoding: generated tokens are base64url and need none.
 * Any other fragment is no invitation.
 */
export function parseInvitation(hash: string): Invitation {
    if (!hash.startsWith(INVITATION_PREFIX)) return { kind: 'none' };
    const token = validToken(hash.slice(INVITATION_PREFIX.length));
    return token === null ? { kind: 'invalid' } : { kind: 'token', token };
}

/**
 * Reads an invitation from the address and removes its fragment from the address
 * bar and the current history entry, before anything is rendered. Any other
 * fragment stays.
 */
export function takeInvitation(): Invitation {
    const invitation = parseInvitation(location.hash);
    if (invitation.kind !== 'none') {
        history.replaceState(history.state, '', location.pathname + location.search);
    }
    return invitation;
}

/** The token held by this page: remembered in the browser or for this page only. */
export class CreationToken {
    #token: string | null;
    #remembered: boolean;

    /** Starts with the remembered token, if a usable one is stored. */
    constructor() {
        this.#token = readStored();
        this.#remembered = this.#token !== null;
    }

    get value(): string | null {
        return this.#token;
    }

    /** Whether the token is stored in the browser, not only held by this page. */
    get remembered(): boolean {
        return this.#remembered;
    }

    /**
     * Holds `token` from now on. With `remember` it is stored too; when storing
     * fails (no `localStorage`), it is held for this page only. Without `remember`
     * the storage is not touched.
     */
    use(token: string, remember: boolean): void {
        this.#token = token;
        this.#remembered = remember && store(token);
    }

    /** Drops the token: from storage and from this page. */
    forget(): void {
        this.#token = null;
        this.#remembered = false;
        try {
            localStorage.removeItem(CREATION_TOKEN_KEY);
        } catch {
            // Nothing could have been stored either.
        }
    }
}

/** The stored token, if any and usable. `localStorage` may throw (site data blocked). */
function readStored(): string | null {
    try {
        const stored = localStorage.getItem(CREATION_TOKEN_KEY);
        return stored === null ? null : validToken(stored);
    } catch {
        return null;
    }
}

/** Stores `token`; `false` if the browser keeps nothing. */
function store(token: string): boolean {
    try {
        localStorage.setItem(CREATION_TOKEN_KEY, token);
        return true;
    } catch {
        return false;
    }
}
