/**
 * Error texts in the page language for failures the pages share: an unreadable
 * link and a refused API call.
 *
 * Core and the server describe errors in English (`LinkError` messages,
 * problem+json `title`/`detail`). The pages never show those; they branch on
 * what is stable - the fragment, the HTTP status and the problem `code` - and
 * take the text from the catalogue.
 */

import { ApiError } from '@capsulebin/core';

import { t, tn } from '../language';

/** Why `parseFragment(fragment)` refused the link: no key at all, or one that cannot be used. */
export function linkErrorText(fragment: string): string {
    return fragment.replace(/^#/, '') === '' ? t('link.noKey') : t('link.unreadable');
}

/** What to tell the user about a failed API call; a general text for anything unexpected. */
export function apiErrorText(err: unknown): string {
    if (!(err instanceof ApiError)) return t('error.generic');
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
