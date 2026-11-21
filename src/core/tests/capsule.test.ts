/**
 * Tests for the capsule file format encoder/decoder.
 */

import { describe, expect, it } from 'vitest';
import {
    Capsule,
    CapsuleError,
    MAX_CAPSULE_BYTES,
    type CapsuleItem,
    padme,
    padTo,
} from '../src/capsule';

/** Converts a Uint8Array to a plain array for deep-equal comparison. */
function toArray(bytes: Uint8Array): number[] {
    return Array.from(bytes);
}

function textItem(overrides: Partial<CapsuleItem> = {}): CapsuleItem {
    return {
        id: '01J00000000000000000000001',
        name: 'notes.md',
        size: 5,
        mediaType: 'text/markdown',
        presentation: 'markdown',
        bytes: new TextEncoder().encode('hello'),
        ...overrides,
    };
}

describe('Capsule construction / encode', () => {
    it('encodes a single text item and decodes it byte-exactly', () => {
        const encoded = new Capsule([textItem()]).encode();
        expect(encoded).toBeInstanceOf(Uint8Array);
        expect(encoded.length).toBeGreaterThan(14);

        const decoded = Capsule.decode(encoded);
        expect(decoded.title).toBeNull();
        expect(decoded.items).toHaveLength(1);
        expect(decoded.items[0]!.name).toBe('notes.md');
        expect(decoded.items[0]!.presentation).toBe('markdown');
        expect(toArray(decoded.items[0]!.bytes)).toEqual(
            toArray(new TextEncoder().encode('hello')),
        );
    });

    it('roundtrips optional language field for code items', () => {
        const id = '01J0000000000000000000000001';
        const capsule = new Capsule([
            {
                id,
                name: 'Rust snippet',
                size: 2,
                mediaType: 'text/plain',
                presentation: 'code',
                language: 'rust',
                bytes: new TextEncoder().encode('//'),
            },
        ]).encode();
        const decoded = Capsule.decode(capsule);
        expect(decoded.items[0]!.presentation).toBe('code');
        expect(decoded.items[0]!.language).toBe('rust');
        expect(decoded.items[0]!.name).toBe('Rust snippet');
    });

    it('encodes and decodes 16 items', () => {
        const items: CapsuleItem[] = [];
        for (let i = 0; i < 16; i++) {
            items.push({
                id: `01J00000000000000000000000${String(i).padStart(2, '0')}`,
                name: `item-${i}`,
                size: 5,
                mediaType: 'text/plain',
                presentation: 'plain',
                bytes: new TextEncoder().encode('hello'),
            });
        }
        const capsule = new Capsule(items).encode();
        const decoded = Capsule.decode(capsule);
        expect(decoded.items).toHaveLength(16);
    });

    it('preserves binary content roundtrip', () => {
        const bytes = new Uint8Array(256);
        for (let i = 0; i < 256; i++) bytes[i] = i;
        const capsule = new Capsule([
            {
                id: '01J0000000000000000000000001',
                name: 'binary.bin',
                size: 256,
                mediaType: 'application/octet-stream',
                presentation: 'download',
                bytes,
            },
        ]).encode();
        const decoded = Capsule.decode(capsule);
        expect(decoded.items).toHaveLength(1);
        expect(decoded.items[0]!.bytes.length).toBe(256);
        for (let i = 0; i < 256; i++) {
            expect(decoded.items[0]!.bytes[i]).toBe(i);
        }
    });

    it('includes the title in the manifest', () => {
        const encoded = new Capsule([textItem()], 'My Capsule').encode();
        const decoded = Capsule.decode(encoded);
        expect(decoded.title).toBe('My Capsule');
    });

    it('rejects an empty item list', () => {
        expect(() => new Capsule([])).toThrow(CapsuleError);
    });

    it('rejects too many items', () => {
        const items = Array.from({ length: 17 }, (_, i) =>
            textItem({ id: `id${i}`, name: `file${i}` }),
        );
        expect(() => new Capsule(items)).toThrow(CapsuleError);
    });

    it('rejects an item name that is too long', () => {
        const longName = 'a'.repeat(256);
        expect(() => new Capsule([textItem({ name: longName })])).toThrow(CapsuleError);
    });

    it('rejects size mismatch', () => {
        expect(() => new Capsule([textItem({ size: 999 })])).toThrow(CapsuleError);
    });

    it('rejects a capsule over the total size limit', () => {
        const bytes = new Uint8Array(MAX_CAPSULE_BYTES + 1);
        expect(() => new Capsule([textItem({ size: bytes.length, bytes })])).toThrow(CapsuleError);
    });

    it('accepts a capsule just under the total size limit', () => {
        // Leaves room for the fixed header and this item's manifest entry.
        const bytes = new Uint8Array(MAX_CAPSULE_BYTES - 1024);
        const encoded = new Capsule([textItem({ size: bytes.length, bytes })]).encode();
        expect(encoded.length).toBeLessThanOrEqual(MAX_CAPSULE_BYTES);
    });
});

describe('Capsule.decode', () => {
    it('rejects data that is too short', () => {
        expect(() => Capsule.decode(new Uint8Array(10))).toThrow(CapsuleError);
    });

    it('rejects wrong magic bytes', () => {
        const bytes = new Uint8Array(20).fill(0);
        expect(() => Capsule.decode(bytes)).toThrow('wrong magic bytes');
    });

    it('rejects an unsupported framing version', () => {
        const magic = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]);
        const out = new Uint8Array(20);
        out.set(magic, 0);
        // Write framing version 99 at offset 8.
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        view.setUint16(8, 99);
        expect(() => Capsule.decode(out)).toThrow('framing version');
    });

    it('rejects invalid UTF-8 in the manifest', () => {
        const magic = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]);
        // Build a minimal header with invalid UTF-8 as manifest.
        const manifestLen = 4;
        const total = 8 + 2 + 4 + manifestLen;
        const out = new Uint8Array(total);
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        out.set(magic, 0);
        view.setUint16(8, 1); // version
        view.setUint32(10, manifestLen);
        // 0xFF is never valid in UTF-8.
        out.set([0xff, 0xff, 0xff, 0xff], 14);
        expect(() => Capsule.decode(out)).toThrow('UTF-8');
    });

    it('rejects a manifest that is not JSON', () => {
        const magic = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]);
        const manifestText = 'not json';
        const manifestBytes = new TextEncoder().encode(manifestText);
        const total = 8 + 2 + 4 + manifestBytes.length;
        const out = new Uint8Array(total);
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        out.set(magic, 0);
        view.setUint16(8, 1);
        view.setUint32(10, manifestBytes.length);
        out.set(manifestBytes, 14);
        expect(() => Capsule.decode(out)).toThrow('JSON');
    });

    it('rejects size sum mismatch', () => {
        // Encode, then cut into the last item: drop the zero padding and one more byte.
        const encoded = new Capsule([textItem()]).encode();
        let end = encoded.length;
        while (encoded[end - 1] === 0) end--;
        const truncated = encoded.subarray(0, end - 1);
        expect(() => Capsule.decode(truncated)).toThrow('item sizes');
    });

    it('pads the encoding to the PADMÉ length with zero bytes', () => {
        const item = textItem();
        const encoded = new Capsule([item]).encode();
        expect(encoded.length).toBe(padme(encoded.length));
        expect(encoded.length).toBeGreaterThanOrEqual(14 + item.size);

        const decoded = Capsule.decode(encoded);
        expect(Array.from(decoded.items[0]!.bytes)).toEqual(Array.from(item.bytes));
    });

    it('still decodes an unpadded capsule', () => {
        const encoded = new Capsule([textItem()]).encode();
        let end = encoded.length;
        while (encoded[end - 1] === 0) end--;
        expect(Capsule.decode(encoded.subarray(0, end)).items).toHaveLength(1);
    });

    it('refuses input above the format limit', () => {
        const encoded = new Capsule([textItem()]).encode();
        const atLimit = new Uint8Array(MAX_CAPSULE_BYTES);
        atLimit.set(encoded);
        expect(Capsule.decode(atLimit).items).toHaveLength(1);

        const above = new Uint8Array(MAX_CAPSULE_BYTES + 1);
        above.set(encoded);
        expect(() => Capsule.decode(above)).toThrow(CapsuleError);
        expect(() => Capsule.decode(above)).toThrow(`${MAX_CAPSULE_BYTES} byte limit`);
        // Refused before the padding is scanned: a non-zero byte in it changes nothing.
        above[above.length - 1] = 1;
        expect(() => Capsule.decode(above)).toThrow(`${MAX_CAPSULE_BYTES} byte limit`);
    });

    it('rejects non-zero bytes after the last item', () => {
        const encoded = new Capsule([textItem()]).encode().slice();
        expect(encoded.at(-1)).toBe(0);
        encoded[encoded.length - 1] = 1;
        expect(() => Capsule.decode(encoded)).toThrow('padding');
    });

    it('handles multiple items with correct boundaries', () => {
        const items: CapsuleItem[] = [
            {
                id: '01J00000000000000000000001',
                name: 'first.txt',
                size: 3,
                mediaType: 'text/plain',
                presentation: 'plain',
                bytes: new Uint8Array([1, 2, 3]),
            },
            {
                id: '01J00000000000000000000002',
                name: 'second.bin',
                size: 4,
                mediaType: 'application/octet-stream',
                presentation: 'download',
                bytes: new Uint8Array([4, 5, 6, 7]),
            },
        ];

        const encoded = new Capsule(items).encode();
        const decoded = Capsule.decode(encoded);

        expect(decoded.items).toHaveLength(2);
        expect(toArray(decoded.items[0]!.bytes)).toEqual([1, 2, 3]);
        expect(toArray(decoded.items[1]!.bytes)).toEqual([4, 5, 6, 7]);
    });

    it('rejects an unknown presentation', () => {
        const manifest = JSON.stringify({
            schema_version: 1,
            title: null,
            items: [
                {
                    id: '01J00000000000000000000001',
                    name: 'test',
                    size: 5,
                    media_type: 'text/plain',
                    presentation: 'html',
                },
            ],
        });
        const manifestBytes = new TextEncoder().encode(manifest);
        const total = 8 + 2 + 4 + manifestBytes.length + 5;
        const out = new Uint8Array(total);
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        const magic = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]);
        out.set(magic, 0);
        view.setUint16(8, 1);
        view.setUint32(10, manifestBytes.length);
        out.set(manifestBytes, 14);
        out.set(new Uint8Array([1, 2, 3, 4, 5]), 14 + manifestBytes.length);
        expect(() => Capsule.decode(out)).toThrow('presentation');
    });

    it('rejects duplicate item ids', () => {
        const manifest = JSON.stringify({
            schema_version: 1,
            title: null,
            items: [
                {
                    id: '01J00000000000000000000001',
                    name: 'a',
                    size: 2,
                    media_type: 'text/plain',
                    presentation: 'plain',
                },
                {
                    id: '01J00000000000000000000001',
                    name: 'b',
                    size: 3,
                    media_type: 'text/plain',
                    presentation: 'plain',
                },
            ],
        });
        const manifestBytes = new TextEncoder().encode(manifest);
        const total = 8 + 2 + 4 + manifestBytes.length + 5;
        const out = new Uint8Array(total);
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        const magic = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]);
        out.set(magic, 0);
        view.setUint16(8, 1);
        view.setUint32(10, manifestBytes.length);
        out.set(manifestBytes, 14);
        out.set(new Uint8Array([1, 2, 3, 4, 5]), 14 + manifestBytes.length);
        expect(() => Capsule.decode(out)).toThrow('duplicate');
    });
});

describe('padme / padTo', () => {
    it.each([
        [0, 0],
        [1, 1],
        [2, 2],
        [9, 10],
        [100, 104],
        [189, 192],
        [1000, 1024],
        [70_000, 71_680],
        [1_000_000, 1_015_808],
    ])('padme(%i) = %i', (length, padded) => {
        expect(padme(length)).toBe(padded);
    });

    it('never adds more than 12 % and never shrinks', () => {
        for (let length = 2; length < 200_000; length += 37) {
            const padded = padme(length);
            expect(padded).toBeGreaterThanOrEqual(length);
            expect(padded / length).toBeLessThanOrEqual(1.125);
        }
    });

    it('padTo never exceeds the capsule limit', () => {
        expect(padTo(MAX_CAPSULE_BYTES - 10)).toBe(MAX_CAPSULE_BYTES);
        expect(padTo(MAX_CAPSULE_BYTES)).toBe(MAX_CAPSULE_BYTES);
    });
});
