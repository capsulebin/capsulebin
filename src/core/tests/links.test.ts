import { describe, expect, it } from 'vitest';
import {
    hasQueryOrFragment,
    isPasteId,
    LinkError,
    parseFragment,
    parseLink,
    pasteIdFromInput,
    trailingLanguage,
} from '../src/links';
import fixture from './fixtures/base-language.json' with { type: 'json' };

const ID = 'AbCdEfGhIjKlMnOpQrStUv';
const SECRET = 'A'.repeat(43); // 32 zero-ish bytes in base64url

describe('parseLink', () => {
    it('parses a share link', () => {
        const link = parseLink(`https://paste.example/p/${ID}#v1.${SECRET}`, 'share');
        expect(link.base).toBe('https://paste.example');
        expect(link.id).toBe(ID);
        expect(link.secret).toHaveLength(32);
        expect(link.kind).toBe('share');
    });

    it('parses a management link', () => {
        const link = parseLink(`http://127.0.0.1:8081/manage/${ID}#v1.${SECRET}`, 'manage');
        expect(link.base).toBe('http://127.0.0.1:8081');
        expect(link.id).toBe(ID);
    });

    it('keeps a path prefix in the base', () => {
        const link = parseLink(`https://example.org/paste/p/${ID}#v1.${SECRET}`, 'share');
        expect(link.base).toBe('https://example.org/paste');
    });

    it.each([
        [`https://x.example/de/p/${ID}#v1.${SECRET}`, 'share', 'https://x.example'],
        [`https://x.example/en/manage/${ID}#v1.${SECRET}`, 'manage', 'https://x.example'],
        [`https://x.example/sub/de/manage/${ID}#v1.${SECRET}`, 'manage', 'https://x.example/sub'],
        [`http://127.0.0.1:8081/de/p/${ID}#v1.${SECRET}`, 'share', 'http://127.0.0.1:8081'],
    ] as const)('drops the language segment from the base of %s', (input, kind, base) => {
        const link = parseLink(input, kind);
        expect(link.base).toBe(base);
        expect(link.id).toBe(ID);
        expect(link.secret).toHaveLength(32);
    });

    it.each([
        // Not a known language: a sub-path that happens to have two letters.
        [`https://x.example/cb/p/${ID}#v1.${SECRET}`, 'https://x.example/cb'],
        // Codes are matched exactly; anything else is a sub-path.
        [`https://x.example/DE/p/${ID}#v1.${SECRET}`, 'https://x.example/DE'],
        [`https://x.example/de-AT/p/${ID}#v1.${SECRET}`, 'https://x.example/de-AT'],
        // Only the segment right before `p` is a language; an earlier one is a sub-path.
        [`https://x.example/de/sub/p/${ID}#v1.${SECRET}`, 'https://x.example/de/sub'],
    ])('keeps %s in the base', (input, base) => {
        expect(parseLink(input, 'share').base).toBe(base);
    });

    it('rejects a language prefix without a page segment', () => {
        expect(() => parseLink(`https://x.example/de/${ID}#v1.${SECRET}`, 'share')).toThrow(
            LinkError,
        );
    });

    it('rejects a management link where a share link is expected, and vice versa', () => {
        expect(() => parseLink(`https://x.test/manage/${ID}#v1.${SECRET}`, 'share')).toThrow(
            LinkError,
        );
        expect(() => parseLink(`https://x.test/p/${ID}#v1.${SECRET}`, 'manage')).toThrow(LinkError);
    });

    it.each([
        ['not a url', 'paste.example/p/x'],
        ['wrong scheme', `ftp://x.test/p/${ID}#v1.${SECRET}`],
        ['short id', `https://x.test/p/abc#v1.${SECRET}`],
        ['id with bad chars', `https://x.test/p/${'+'.repeat(22)}#v1.${SECRET}`],
        ['no fragment', `https://x.test/p/${ID}`],
        ['wrong fragment version', `https://x.test/p/${ID}#v2.${SECRET}`],
        ['short secret', `https://x.test/p/${ID}#v1.AAAA`],
    ])('rejects %s', (_label, link) => {
        expect(() => parseLink(link, 'share')).toThrow(LinkError);
    });
});

describe('trailingLanguage', () => {
    it.each(fixture.vectors)('finds $language at the end of $base', ({ base, language }) => {
        expect(trailingLanguage(base)).toBe(language);
    });

    // The reason for the rule: a link below such a base loses the code to
    // `parseLink`, so its capability would go to the shortened base.
    it.each(fixture.vectors)('agrees with parseLink about $base', ({ base, language }) => {
        const url = new URL(base);
        const withoutSlashes = `${url.origin}${url.pathname.replace(/\/+$/, '')}`;
        const parsed = parseLink(`${withoutSlashes}/manage/${ID}#v1.${SECRET}`, 'manage');
        expect(parsed.base === withoutSlashes).toBe(language === null);
    });

    it.each(['example.org/de', 'ftp://example.org/de', 'https://'])('rejects %s', (base) => {
        expect(() => trailingLanguage(base)).toThrow(LinkError);
    });
});

describe('hasQueryOrFragment', () => {
    it.each(fixture.query_or_fragment)('is $refused for $base', ({ base, refused }) => {
        expect(hasQueryOrFragment(base)).toBe(refused);
    });

    // The reason for the rule: clients append the request path to the base as
    // text, so behind a query or fragment it never reaches the URL's path.
    it.each(fixture.query_or_fragment)(
        'agrees with the URL parser about $base',
        ({ base, refused }) => {
            const appended = new URL(`${base}/api/v1/pastes`);
            expect(appended.pathname.endsWith('/api/v1/pastes')).toBe(!refused);
        },
    );
});

describe('parseFragment', () => {
    it('accepts the fragment with or without the leading #', () => {
        expect(parseFragment(`#v1.${SECRET}`)).toHaveLength(32);
        expect(parseFragment(`v1.${SECRET}`)).toHaveLength(32);
    });

    it('rejects an empty fragment', () => {
        expect(() => parseFragment('')).toThrow(/no key/);
        expect(() => parseFragment('#')).toThrow(/no key/);
    });
});

describe('isPasteId', () => {
    it('accepts exactly 22 base64url characters', () => {
        expect(isPasteId(ID)).toBe(true);
        expect(isPasteId(ID + 'x')).toBe(false);
        expect(isPasteId('')).toBe(false);
    });
});

describe('pasteIdFromInput', () => {
    it.each([
        [ID, ID],
        [`  ${ID}  `, ID],
        [`https://paste.example/p/${ID}`, ID],
        [`https://paste.example/p/${ID}#v1.${SECRET}`, ID],
        [`https://paste.example/manage/${ID}/`, ID],
        [`https://paste.example/de/p/${ID}#v1.${SECRET}`, ID],
        [`https://paste.example/sub/en/manage/${ID}`, ID],
        ['https://paste.example/p/short', null],
        ['', null],
        // An id-shaped fragment is never picked up.
        [`https://paste.example/p/#${ID}`, null],
    ])('%j → %j', (input, expected) => {
        expect(pasteIdFromInput(input)).toBe(expected);
    });
});
