import { describe, expect, it } from 'vitest';
import { isPasteId, LinkError, parseFragment, parseLink, pasteIdFromInput } from '../src/links';

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
        ['https://paste.example/p/short', null],
        ['', null],
        // An id-shaped fragment is never picked up.
        [`https://paste.example/p/#${ID}`, null],
    ])('%j → %j', (input, expected) => {
        expect(pasteIdFromInput(input)).toBe(expected);
    });
});
