/**
 * src/expiry-options.ts: the create page's expiry policy from its meta, the
 * offered options, their labels and the preselected default.
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
    BUILT_IN_EXPIRY,
    type ExpiryPolicy,
    expiryLabel,
    expiryPolicy,
    expiryValues,
    fillExpirySelect,
} from '../src/expiry-options';
import de from '../src/i18n/de.json';
import en from '../src/i18n/en.json';
import { tn } from '../src/language';
import { setPageLanguage } from './page-language';

const STEPS = ['3600', '86400', '604800', '2592000', '7776000'];

function policy(defaultSeconds: number, maximumSeconds: number, permanent = false): ExpiryPolicy {
    return { defaultSeconds, maximumSeconds, permanent };
}

/** Puts `<meta name="capsulebin-expiry">` with `attributes` into the head. */
function meta(attributes: Record<string, string>): void {
    const element = document.createElement('meta');
    element.name = 'capsulebin-expiry';
    for (const [name, value] of Object.entries(attributes)) element.setAttribute(name, value);
    document.head.replaceChildren(element);
}

beforeEach(async () => {
    await setPageLanguage('en');
});

afterEach(() => {
    document.head.replaceChildren();
});

// AC5
describe('expiry options', () => {
    it('are the five steps without "No expiry" for the built-in defaults', () => {
        expect(expiryValues(BUILT_IN_EXPIRY)).toEqual(STEPS);
    });

    it('add a maximum and a default that are no step, in ascending order', () => {
        expect(expiryValues(policy(259_200, 1_209_600))).toEqual([
            '3600',
            '86400',
            '259200',
            '604800',
            '1209600',
        ]);
    });

    it('are only the maximum when it is the default and below every step', () => {
        expect(expiryValues(policy(1_800, 1_800))).toEqual(['1800']);
    });

    it('end with "No expiry" when capsules without expiry are allowed', () => {
        expect(expiryValues(policy(604_800, 7_776_000, true))).toEqual([...STEPS, '']);
    });

    it('offer no step above the maximum', () => {
        expect(expiryValues(policy(86_400, 172_800))).toEqual(['3600', '86400', '172800']);
    });
});

// AC6
describe('expiry labels', () => {
    it('keep the steps’ own labels and "No expiry"', () => {
        expect(STEPS.map(expiryLabel)).toEqual([
            en['create.expiry.hour'],
            en['create.expiry.day'],
            en['create.expiry.week'],
            en['create.expiry.30days'],
            en['create.expiry.90days'],
        ]);
        expect(expiryLabel('')).toBe(en['expiry.none']);
    });

    it.each([
        ['1209600', '14 days', 'Nach 14 Tagen'],
        ['172800', '2 days', 'Nach 2 Tagen'],
        ['129600', '36 hours', 'Nach 36 Stunden'],
        ['7200', '2 hours', 'Nach 2 Stunden'],
        ['1800', '30 minutes', 'Nach 30 Minuten'],
        ['60', '1 minute', 'Nach 1 Minute'],
        ['90', '90 seconds', 'Nach 90 Sekunden'],
        ['1', '1 second', 'Nach 1 Sekunde'],
    ])('name %s seconds in the largest exact unit', async (value, english, german) => {
        expect(expiryLabel(value)).toBe(english);
        await setPageLanguage('de');
        expect(expiryLabel(value)).toBe(german);
    });

    it('keep the steps’ own labels in German', async () => {
        await setPageLanguage('de');
        expect(expiryLabel('86400')).toBe(de['create.expiry.day']);
        expect(expiryLabel('')).toBe(de['expiry.none']);
    });

    it.each([
        ['en', en],
        ['de', de],
    ] as const)('take the steps’ form in %s', async (language, catalogue) => {
        await setPageLanguage(language);
        expect(tn('create.expiry.hours', 1)).toBe(catalogue['create.expiry.hour']);
        expect(tn('create.expiry.days', 1)).toBe(catalogue['create.expiry.day']);
        expect(tn('create.expiry.days', 30)).toBe(catalogue['create.expiry.30days']);
        expect(tn('create.expiry.days', 90)).toBe(catalogue['create.expiry.90days']);
    });
});

// AC7
describe('the expiry select', () => {
    it('selects the default, the only option marked as default', () => {
        const select = document.createElement('select');
        fillExpirySelect(select, policy(259_200, 1_209_600, true));
        expect([...select.options].map((option) => [option.value, option.textContent])).toEqual([
            ['3600', en['create.expiry.hour']],
            ['86400', en['create.expiry.day']],
            ['259200', '3 days'],
            ['604800', en['create.expiry.week']],
            ['1209600', '14 days'],
            ['', en['expiry.none']],
        ]);
        expect(select.value).toBe('259200');
        expect(
            [...select.options].filter((option) => option.defaultSelected).map((o) => o.value),
        ).toEqual(['259200']);
    });

    it('replaces options it held before', () => {
        const select = document.createElement('select');
        fillExpirySelect(select, policy(1_800, 1_800));
        fillExpirySelect(select, BUILT_IN_EXPIRY);
        expect([...select.options].map((option) => option.value)).toEqual(STEPS);
        expect(select.value).toBe('604800');
    });
});

// AC8
describe('expiry policy', () => {
    it('is read from the meta', () => {
        meta({ 'data-default': '259200', 'data-maximum': '1209600', 'data-permanent': 'true' });
        expect(expiryPolicy()).toEqual(policy(259_200, 1_209_600, true));
        meta({ 'data-default': '1800', 'data-maximum': '1800', 'data-permanent': 'false' });
        expect(expiryPolicy()).toEqual(policy(1_800, 1_800, false));
    });

    it('is the built-in default without a meta', () => {
        expect(expiryPolicy()).toEqual(BUILT_IN_EXPIRY);
    });

    const valid = { 'data-default': '259200', 'data-maximum': '1209600', 'data-permanent': 'true' };

    it.each([
        ['a missing default', { 'data-default': null }],
        ['a missing maximum', { 'data-maximum': null }],
        ['a missing permanent', { 'data-permanent': null }],
        ['an empty default', { 'data-default': '' }],
        ['a zero default', { 'data-default': '0' }],
        ['a negative maximum', { 'data-maximum': '-1' }],
        ['an exponent', { 'data-default': '1e3' }],
        ['a leading space', { 'data-default': ' 5' }],
        ['a hexadecimal number', { 'data-maximum': '0x10' }],
        ['a fraction', { 'data-default': '1.5' }],
        ['an unsafe integer', { 'data-maximum': '9007199254740993' }],
        ['an unfilled token', { 'data-default': '%%capsulebin:expiry-default%%' }],
        ['a default above the maximum', { 'data-default': '1209601' }],
        ['another permanent value', { 'data-permanent': 'yes' }],
        ['an upper-case permanent', { 'data-permanent': 'TRUE' }],
    ])('is the built-in default with %s', (_case, change) => {
        const attributes: Record<string, string> = {};
        for (const [name, value] of Object.entries({ ...valid, ...change })) {
            if (value !== null) attributes[name] = value;
        }
        meta(attributes);
        expect(expiryPolicy()).toEqual(BUILT_IN_EXPIRY);
    });
});

// AC5, edge cases derived from the criterion: a maximum or default that is a
// step is offered once, and the list never holds a value above the maximum.
describe('expiry options at the edges', () => {
    it('offer a maximum that is a step once', () => {
        expect(expiryValues(policy(604_800, 2_592_000))).toEqual([
            '3600',
            '86400',
            '604800',
            '2592000',
        ]);
    });

    it('offer a default that is a step once, below a maximum that is none', () => {
        expect(expiryValues(policy(86_400, 1_209_600, true))).toEqual([
            '3600',
            '86400',
            '604800',
            '1209600',
            '',
        ]);
    });

    it('offer the steps below the maximum plus maximum and default, nothing above', () => {
        const values = expiryValues(policy(5_400, 10_000_000));
        expect(values).toEqual([
            '3600',
            '5400',
            '86400',
            '604800',
            '2592000',
            '7776000',
            '10000000',
        ]);
        expect(new Set(values).size).toBe(values.length);
    });
});
