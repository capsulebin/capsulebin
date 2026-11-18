import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { formatExpiry, formatSize } from '../src/helpers/format';

describe('formatSize', () => {
    it('formats bytes below 1 KiB', () => {
        expect(formatSize(0)).toBe('0 B');
        expect(formatSize(1023)).toBe('1023 B');
    });

    it('formats kibibytes', () => {
        expect(formatSize(1024)).toBe('1.0 KiB');
        expect(formatSize(1536)).toBe('1.5 KiB');
    });

    it('formats mebibytes', () => {
        expect(formatSize(1024 * 1024)).toBe('1.0 MiB');
        expect(formatSize(10 * 1024 * 1024)).toBe('10.0 MiB');
    });
});

describe('formatExpiry', () => {
    const now = new Date('2026-08-05T12:00:00Z');

    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(now);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('reports no expiry', () => {
        expect(formatExpiry(null)).toBe('No expiry');
    });

    it('reports an already-passed expiry as expired', () => {
        const past = Math.floor(now.getTime() / 1000) - 10;
        expect(formatExpiry(past)).toBe('Expired');
    });

    it('reports minutes for a near expiry', () => {
        const soon = Math.floor(now.getTime() / 1000) + 5 * 60;
        expect(formatExpiry(soon)).toBe('Expires in 5 minutes');
    });

    it('uses singular minute', () => {
        const soon = Math.floor(now.getTime() / 1000) + 60;
        expect(formatExpiry(soon)).toBe('Expires in 1 minute');
    });

    it('reports hours once past an hour', () => {
        const later = Math.floor(now.getTime() / 1000) + 3 * 3600;
        expect(formatExpiry(later)).toBe('Expires in 3 hours');
    });

    it('reports days once past a day', () => {
        const later = Math.floor(now.getTime() / 1000) + 6 * 86400;
        expect(formatExpiry(later)).toBe('Expires in 6 days');
    });

    it('uses singular day', () => {
        const later = Math.floor(now.getTime() / 1000) + 86400;
        expect(formatExpiry(later)).toBe('Expires in 1 day');
    });
});
