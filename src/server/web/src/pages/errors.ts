/**
 * Error texts in the page language for failures the pages share: an unreadable
 * link and a refused API call.
 *
 * Core and the server describe errors in English (`LinkError` messages,
 * problem+json `title`/`detail`). The pages never show those; they branch on
 * what is stable - the fragment, the HTTP status and the problem `code` - and
 * take the text from the catalogue.
 */

import { ApiError, INVALID_RESPONSE, REDIRECT_REFUSED } from '@capsulebin/core';

import { t, tn } from '../language';

/** Why `parseFragment(fragment)` refused the link: no key at all, or one that cannot be used. */
export function linkErrorText(fragment: string): string {
    return fragment.replace(/^#/, '') === '' ? t('link.noKey') : t('link.unreadable');
}

/**
 * Whether a create was refused for its creation token (missing or wrong). In
 * `token` mode the create page asks for a token instead of showing
 * {@link apiErrorText}'s text.
 */
export function isTokenRefused(err: unknown): boolean {
    return err instanceof ApiError && err.code === 'unauthorized';
}

/**
 * Whether a failed create may still have stored the capsule: no usable answer
 * (network error, timeout, an oversized or malformed success), a server error
 * or a `429`. Sending the same sealed capsule again is then safe (a stored one
 * is answered as a replay, ADR-011). A refused redirect is a configuration
 * problem, any other status a definite answer; anything not from the API
 * failed before a request was sent.
 */
export function isUnclearOutcome(err: unknown): boolean {
    if (!(err instanceof ApiError) || err.code === REDIRECT_REFUSED) return false;
    return err.status === 0 || err.status === 429 || err.status >= 500;
}

/** What to tell the user about a failed API call; a general text for anything unexpected. */
export function apiErrorText(err: unknown): string {
    if (!(err instanceof ApiError)) return t('error.generic');
    if (err.code === INVALID_RESPONSE) return t('error.invalidResponse');
    if (err.status === 0) return t('error.network');
    switch (err.code) {
        case 'rate_limited':
            return err.retryAfter
                ? tn('error.rateLimitedFor', err.retryAfter)
                : t('error.rateLimited');
        case 'payload_too_large':
            return t('error.payloadTooLarge');
        case 'expiry_too_long':
            return t('error.expiryTooLong');
        case 'permanent_not_allowed':
            return t('error.permanentNotAllowed');
        case 'unauthorized':
        case 'creation_disabled':
            return t('error.creationClosed');
        case 'internal_error':
            return t('error.server');
        default:
            return t('error.generic');
    }
}
