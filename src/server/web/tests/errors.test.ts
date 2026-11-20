/**
 * src/pages/errors.ts: the texts for an unreadable link and a refused API call
 * come from the catalogue of the page language, never from core's or the
 * server's English messages.
 */

import { describe, expect, it } from 'vitest';

import { ApiError } from '@capsulebin/core';

import de from '../src/i18n/de.json';
import en from '../src/i18n/en.json';
import { apiErrorText, linkErrorText } from '../src/pages/errors';
import { setPageLanguage } from './page-language';

describe('link errors', () => {
    it('tell a missing key from an unusable one', async () => {
        await setPageLanguage('de');
        expect(linkErrorText('')).toBe(de['link.noKey']);
        expect(linkErrorText('#')).toBe(de['link.noKey']);
        expect(linkErrorText('#v1.short')).toBe(de['link.unreadable']);
    });
});

describe('API errors', () => {
    const problem = (status: number, code: string | null, retryAfter: number | null = null) =>
        new ApiError('English title', status, code, retryAfter);

    it('are explained by code', async () => {
        await setPageLanguage('en');
        expect(apiErrorText(problem(413, 'payload_too_large'))).toBe(en['error.payloadTooLarge']);
        expect(apiErrorText(problem(400, 'expiry_too_long'))).toBe(en['error.expiryTooLong']);
        expect(apiErrorText(problem(400, 'permanent_not_allowed'))).toBe(
            en['error.permanentNotAllowed'],
        );
        expect(apiErrorText(problem(401, 'unauthorized'))).toBe(en['error.creationClosed']);
        expect(apiErrorText(problem(403, 'creation_disabled'))).toBe(en['error.creationClosed']);
        expect(apiErrorText(problem(500, 'internal_error'))).toBe(en['error.server']);
    });

    it('say how long to wait after a rate limit, if the server said so', async () => {
        await setPageLanguage('de');
        expect(apiErrorText(problem(429, 'rate_limited', 1))).toBe(
            'Zu viele Anfragen. Versuch es in 1 Sekunde noch einmal.',
        );
        expect(apiErrorText(problem(429, 'rate_limited'))).toBe(de['error.rateLimited']);
    });

    it('fall back to a general text, and never show the server title', async () => {
        await setPageLanguage('de');
        expect(apiErrorText(problem(0, null))).toBe(de['error.network']);
        expect(apiErrorText(problem(409, 'id_conflict'))).toBe(de['error.generic']);
        expect(apiErrorText(problem(418, null))).toBe(de['error.generic']);
        expect(apiErrorText(new Error('English message'))).toBe(de['error.generic']);
    });
});
