import { describe, expect, it } from 'vitest';
import { generateUlid } from '../src/helpers/ulid';

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';

describe('generateUlid', () => {
    it('produces exactly 26 characters', () => {
        expect(generateUlid()).toHaveLength(26);
    });

    it('uses only Crockford base32 characters', () => {
        for (let i = 0; i < 100; i++) {
            const ulid = generateUlid();
            for (const ch of ulid) {
                expect(CROCKFORD).toContain(ch);
            }
        }
    });

    it('produces monotonically increasing timestamps', () => {
        const a = generateUlid();
        const b = generateUlid();
        // The first 10 characters encode the timestamp — they should sort
        // lexicographically in generation order.
        expect(a.substring(0, 10) <= b.substring(0, 10)).toBe(true);
    });

    it('produces unique values', () => {
        const ids = new Set<string>();
        for (let i = 0; i < 100; i++) {
            ids.add(generateUlid());
        }
        expect(ids.size).toBe(100);
    });
});
