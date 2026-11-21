/**
 * src/creation-token.ts: the creation mode from the page, the accepted token
 * values, the invitation link's fragment and where the token is held.
 */

import { afterEach, describe, expect, it } from 'vitest';

import {
    CREATION_TOKEN_KEY,
    CreationToken,
    MAX_TOKEN_CHARS,
    creationMode,
    parseInvitation,
    takeInvitation,
    validToken,
} from '../src/creation-token';

afterEach(() => {
    localStorage.clear();
    history.replaceState(null, '', '/');
    document.head.replaceChildren();
});

describe('creation mode', () => {
    function meta(content: string | null): void {
        const element = document.createElement('meta');
        element.name = 'capsulebin-creation';
        if (content !== null) element.setAttribute('content', content);
        document.head.replaceChildren(element);
    }

    it.each(['public', 'token', 'disabled'] as const)('reads %s from the meta', (mode) => {
        meta(mode);
        expect(creationMode()).toBe(mode);
    });

    it('is public without a meta or with an unknown value', () => {
        expect(creationMode()).toBe('public');
        meta(null);
        expect(creationMode()).toBe('public');
        meta('%%capsulebin:creation%%');
        expect(creationMode()).toBe('public');
        meta('TOKEN');
        expect(creationMode()).toBe('public');
    });
});

describe('token values', () => {
    it('accepts visible ASCII after trimming', () => {
        expect(validToken('abc_DEF-123')).toBe('abc_DEF-123');
        expect(validToken('  abc \n')).toBe('abc');
        expect(validToken('!~')).toBe('!~');
        expect(validToken('x')).toBe('x');
        expect(validToken('x'.repeat(MAX_TOKEN_CHARS))).toBe('x'.repeat(MAX_TOKEN_CHARS));
    });

    it.each([
        ['empty', ''],
        ['only whitespace', ' \t '],
        ['a space inside', 'ab cd'],
        ['a tab inside', 'ab\tcd'],
        ['too long', 'x'.repeat(MAX_TOKEN_CHARS + 1)],
        ['non-ASCII', 'tök'],
        ['DEL', 'ab\u007f'],
        ['a control character', 'ab\u0000'],
        ['a no-break space', 'ab cd'],
    ])('refuses %s', (_case, value) => {
        expect(validToken(value)).toBeNull();
    });
});

describe('invitation fragment', () => {
    it('takes the token as it is, without percent-decoding', () => {
        expect(parseInvitation('#token=abc')).toEqual({ kind: 'token', token: 'abc' });
        expect(parseInvitation('#token=a%20b')).toEqual({ kind: 'token', token: 'a%20b' });
        expect(parseInvitation('#token=a=b')).toEqual({ kind: 'token', token: 'a=b' });
    });

    it('calls an unusable value invalid', () => {
        expect(parseInvitation('#token=')).toEqual({ kind: 'invalid' });
        expect(parseInvitation(`#token=${'x'.repeat(MAX_TOKEN_CHARS + 1)}`)).toEqual({
            kind: 'invalid',
        });
    });

    it.each(['', '#', '#token', '#tokens=abc', '#Token=abc', '#v1.secret', '#x&token=abc'])(
        'is no invitation for %j',
        (hash) => {
            expect(parseInvitation(hash)).toEqual({ kind: 'none' });
        },
    );

    it('removes an invitation from the address and the history entry', () => {
        history.replaceState({ kept: true }, '', '/de/?q=1#token=abc');
        const length = history.length;
        expect(takeInvitation()).toEqual({ kind: 'token', token: 'abc' });
        expect(location.hash).toBe('');
        expect(location.href).toBe(`${location.origin}/de/?q=1`);
        expect(history.length).toBe(length);
        expect(history.state).toEqual({ kept: true });
    });

    it('removes an invalid invitation too', () => {
        history.replaceState(null, '', '/#token=');
        expect(takeInvitation()).toEqual({ kind: 'invalid' });
        expect(location.href).toBe(`${location.origin}/`);
    });

    it('leaves any other fragment alone', () => {
        history.replaceState(null, '', '/#section');
        expect(takeInvitation()).toEqual({ kind: 'none' });
        expect(location.hash).toBe('#section');
    });
});

describe('held token', () => {
    it('starts with a usable stored token', () => {
        localStorage.setItem(CREATION_TOKEN_KEY, 'stored');
        const token = new CreationToken();
        expect(token.value).toBe('stored');
        expect(token.remembered).toBe(true);
    });

    it('ignores an unusable stored value', () => {
        localStorage.setItem(CREATION_TOKEN_KEY, 'not a token');
        expect(new CreationToken().value).toBeNull();
    });

    it('stores a remembered token', () => {
        const token = new CreationToken();
        expect(token.value).toBeNull();
        token.use('abc', true);
        expect(token.value).toBe('abc');
        expect(token.remembered).toBe(true);
        expect(localStorage.getItem(CREATION_TOKEN_KEY)).toBe('abc');
    });

    it('never writes a token held for the page only', () => {
        const token = new CreationToken();
        token.use('abc', false);
        expect(token.value).toBe('abc');
        expect(token.remembered).toBe(false);
        expect(localStorage.getItem(CREATION_TOKEN_KEY)).toBeNull();
    });

    it('forgets from storage and from the page', () => {
        const token = new CreationToken();
        token.use('abc', true);
        token.forget();
        expect(token.value).toBeNull();
        expect(token.remembered).toBe(false);
        expect(localStorage.getItem(CREATION_TOKEN_KEY)).toBeNull();
    });

    describe('when localStorage throws', () => {
        const descriptor = Object.getOwnPropertyDescriptor(window, 'localStorage')!;

        afterEach(() => {
            Object.defineProperty(window, 'localStorage', descriptor);
        });

        it('holds the token for the page only', () => {
            Object.defineProperty(window, 'localStorage', {
                configurable: true,
                get() {
                    throw new DOMException('blocked', 'SecurityError');
                },
            });
            const token = new CreationToken();
            expect(token.value).toBeNull();
            token.use('abc', true);
            expect(token.value).toBe('abc');
            expect(token.remembered).toBe(false);
            token.forget();
            expect(token.value).toBeNull();
        });
    });
});
