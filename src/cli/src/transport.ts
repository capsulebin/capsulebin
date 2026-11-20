/**
 * Which servers the CLI talks to, and when two server URLs name the same one.
 *
 * Every request carries a capability (creation token, describe or reveal
 * token, delete secret), so it only goes over HTTPS. Plain HTTP is allowed
 * for loopback — exactly `localhost`, an IPv4 address in `127.0.0.0/8` and
 * `[::1]` — and otherwise only on explicit opt-in (`--allow-http` or
 * `CAPSULEBIN_ALLOW_HTTP=1`). Names under `.localhost` and private networks
 * are real networks and need the opt-in like any other host.
 */

import { usage } from './exit';
import type { Io } from './io';

const ALLOW_HTTP_ENV = 'CAPSULEBIN_ALLOW_HTTP';

/**
 * Whether plain HTTP outside loopback was opted into: `--allow-http`, or
 * `CAPSULEBIN_ALLOW_HTTP=1`. Any other non-empty value of the variable is a
 * usage error, so a typo such as `true` does not silently keep the refusal.
 */
export function allowHttp(io: Io, flag: boolean): boolean {
    const fromEnv = io.env[ALLOW_HTTP_ENV] ?? '';
    if (fromEnv !== '' && fromEnv !== '1') {
        throw usage(`${ALLOW_HTTP_ENV} must be 1 or empty`);
    }
    return flag || fromEnv === '1';
}

function isLoopback(url: URL): boolean {
    // The URL parser has already lower-cased the host and normalized IPv4
    // literals (`127.1` → `127.0.0.1`), so this compares canonical forms.
    const host = url.hostname;
    return host === 'localhost' || host === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(host);
}

/**
 * Refuses a server base (`what` names it in the message) that would carry
 * capabilities over plain HTTP to a host other than loopback, unless
 * `allowHttp`. `base` must already be a valid http(s) URL.
 */
export function checkTransport(base: string, allowHttp: boolean, what: string): void {
    const url = new URL(base);
    if (url.protocol === 'https:' || allowHttp || isLoopback(url)) return;
    throw usage(
        `${what} ${url.origin} uses plain HTTP, which would send secrets unencrypted; ` +
            `use https, or pass --allow-http (or set ${ALLOW_HTTP_ENV}=1) to allow it`,
    );
}

/**
 * A server base in the form two bases can be compared in: origin (scheme,
 * lower-case host, port unless the default) plus the path without trailing
 * slashes. A link's base comes from `parseLink`, which has already removed
 * the language segment; a `--server` base cannot end in one.
 */
export function normalizeBase(base: string): string {
    const url = new URL(base);
    return url.origin + url.pathname.replace(/\/+$/, '');
}
