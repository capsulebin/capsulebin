import { describe, expect, it } from 'vitest';
import { isKnownLanguage, KNOWN_LANGUAGES } from '../src/languages';

describe('languages', () => {
    it('lists English and German', () => {
        expect(KNOWN_LANGUAGES).toEqual(['en', 'de']);
    });

    it.each([
        ['en', true],
        ['de', true],
        ['DE', false],
        ['de-AT', false],
        ['cb', false],
        ['', false],
    ])('isKnownLanguage(%j) is %j', (code, expected) => {
        expect(isKnownLanguage(code)).toBe(expected);
    });
});
