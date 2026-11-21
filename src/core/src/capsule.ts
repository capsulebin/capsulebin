/**
 * Capsule File Format v1: binary framing, JSON manifest and raw item bytes.
 *
 * Layout on disk (everything is big-endian):
 *
 *   Byte 0–7     Magic: ASCII "CAPSULE\0"
 *   Byte 8–9     Framing version: u16, value 1
 *   Byte 10–13   Manifest length: u32
 *   Byte 14–N    Manifest: exactly manifest_length bytes of UTF-8 JSON
 *   Byte N+1…    Items concatenated in manifest order, no delimiter
 *   then         Padding: 0x00 bytes up to PADMÉ(length), see padTo()
 *
 * Item boundaries are determined exclusively from the sizes declared in the
 * manifest. There is no delimiter between items because any byte sequence could
 * appear in a binary file, and escaping would be fragile.
 */

const MAGIC = new Uint8Array([0x43, 0x41, 0x50, 0x53, 0x55, 0x4c, 0x45, 0x00]); // "CAPSULE\0"
const FRAMING_VERSION = 1;
const MANIFEST_SCHEMA_VERSION = 1;

/** Allowed presentation values. */
const PRESENTATIONS = new Set(['plain', 'code', 'markdown', 'download']);

/** Maximum number of items per capsule. */
const MAX_ITEMS = 16;

/** Maximum size of the JSON manifest in bytes. */
const MAX_MANIFEST_BYTES = 64 * 1024; // 64 KiB

/** Maximum total capsule size in bytes: mirrors the server's default
 *  `pastes.max_plaintext_size` (10 MiB). The server remains authoritative —
 *  it may be configured to a different value — but this lets the client
 *  reject an oversized capsule before spending time encrypting it, instead
 *  of only learning about the limit from a 413 response. */
const MAX_CAPSULE_BYTES = 10 * 1024 * 1024;

/** Fixed framing overhead: magic (8) + version (2) + manifest length (4). */
const HEADER_BYTES = 14;

/** Budget for raw item content once the fixed header and a worst-case
 *  manifest are reserved: a capsule whose items fit this budget is
 *  guaranteed to encode within `MAX_CAPSULE_BYTES`. */
const MAX_ITEM_BYTES = MAX_CAPSULE_BYTES - MAX_MANIFEST_BYTES - HEADER_BYTES;

export { MAX_ITEMS, MAX_CAPSULE_BYTES, MAX_ITEM_BYTES };

/**
 * PADMÉ (Nikitin et al., "Reducing Metadata Leakage from Encrypted Files and
 * Communication with PURBs", 2019): rounds a length up so that only
 * O(log log L) bits of it remain, at an overhead of at most ~12 % (less for
 * larger sizes). Lengths below 2 stay as they are.
 */
export function padme(length: number): number {
    if (length < 2) return length;
    const e = Math.floor(Math.log2(length));
    const s = Math.floor(Math.log2(e)) + 1;
    const lastBits = e - s;
    if (lastBits <= 0) return length;
    const mask = 2 ** lastBits - 1;
    // Integer arithmetic on safe integers; lengths here stay far below 2^53.
    return Math.ceil(length / (mask + 1)) * (mask + 1);
}

/** The padded length of an encoded capsule of `length` unpadded bytes. */
export function padTo(length: number): number {
    return Math.max(length, Math.min(padme(length), MAX_CAPSULE_BYTES));
}

/** Maximum length of an item name in UTF-8 bytes. */
const MAX_NAME_BYTES = 255;

export interface CapsuleItem {
    /** Unique id within this capsule (ULID string). */
    id: string;
    /** Display and download name. Never a server-side filesystem path. */
    name: string;
    /** Byte length of this item. */
    size: number;
    /** MIME type for download. */
    mediaType: string;
    /** Presentation hint: "plain", "code", "markdown" or "download". */
    presentation: string;
    /** Optional language hint for code items. */
    language?: string;
    /** Raw item bytes. */
    bytes: Uint8Array;
}

interface CapsuleManifest {
    schema_version: number;
    title: string | null;
    items: CapsuleManifestItem[];
}

interface CapsuleManifestItem {
    id: string;
    name: string;
    size: number;
    media_type: string;
    presentation: string;
    /** Optional language hint for code items (forward-compat: unknown optional fields are ignored). */
    language?: string;
}

export class CapsuleError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'CapsuleError';
    }
}

/**
 * A capsule's items and title, validated so that it is always encodable.
 *
 * Build one from items you have in hand (validates eagerly, at construction —
 * an invalid item count, name or size throws immediately rather than only
 * once {@link encode} is called), or read one back via {@link decode}.
 */
export class Capsule {
    public readonly title: string | null;
    public readonly items: readonly CapsuleItem[];
    #manifestBytes: Uint8Array;

    /**
     * Validates item count, name lengths and manifest/total size before
     * accepting the items. The caller is responsible for ensuring
     * `item.bytes.length === item.size`.
     */
    constructor(items: CapsuleItem[], title: string | null = null) {
        if (items.length === 0) {
            throw new CapsuleError('a capsule must contain at least one item');
        }
        if (items.length > MAX_ITEMS) {
            throw new CapsuleError(
                `a capsule may contain at most ${MAX_ITEMS} items, got ${items.length}`,
            );
        }

        // Build the manifest items, validating as we go.
        const manifestItems: CapsuleManifestItem[] = [];
        for (const item of items) {
            const nameBytes = new TextEncoder().encode(item.name);
            if (nameBytes.length > MAX_NAME_BYTES) {
                throw new CapsuleError(
                    `item name exceeds ${MAX_NAME_BYTES} UTF-8 bytes: ${item.name}`,
                );
            }
            if (item.size !== item.bytes.length) {
                throw new CapsuleError(
                    `item ${item.id}: declared size ${item.size} does not match byte length ${item.bytes.length}`,
                );
            }
            manifestItems.push({
                id: item.id,
                name: item.name,
                size: item.size,
                media_type: item.mediaType,
                presentation: item.presentation,
                ...(item.language ? { language: item.language } : {}),
            });
        }

        const manifest: CapsuleManifest = {
            schema_version: MANIFEST_SCHEMA_VERSION,
            title,
            items: manifestItems,
        };

        const manifestBytes = new TextEncoder().encode(JSON.stringify(manifest));
        if (manifestBytes.length > MAX_MANIFEST_BYTES) {
            throw new CapsuleError(
                `manifest is ${manifestBytes.length} bytes, exceeds limit of ${MAX_MANIFEST_BYTES}`,
            );
        }

        let totalSize = HEADER_BYTES + manifestBytes.length;
        for (const item of items) {
            totalSize += item.bytes.length;
        }
        if (totalSize > MAX_CAPSULE_BYTES) {
            throw new CapsuleError(
                `capsule is ${totalSize} bytes, exceeds the ${MAX_CAPSULE_BYTES} byte limit`,
            );
        }

        this.title = title;
        this.items = items;
        this.#manifestBytes = manifestBytes;
    }

    /**
     * Decodes a capsule from its binary representation.
     *
     * Validates magic, framing version, manifest JSON and item size
     * consistency before returning any data. Throws {@link CapsuleError} on
     * any mismatch, and on input longer than `MAX_CAPSULE_BYTES` before
     * looking at it: no encoder produces more, because padding is capped there.
     */
    public static decode(bytes: Uint8Array): Capsule {
        if (bytes.length > MAX_CAPSULE_BYTES) {
            throw new CapsuleError(
                `capsule is ${bytes.length} bytes, exceeds the ${MAX_CAPSULE_BYTES} byte limit`,
            );
        }
        if (bytes.length < 14) {
            throw new CapsuleError('capsule too short: missing header');
        }

        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        let offset = 0;

        // Magic.
        for (let i = 0; i < MAGIC.length; i++) {
            if (bytes[offset + i] !== MAGIC[i]) {
                throw new CapsuleError('not a capsule file: wrong magic bytes');
            }
        }
        offset += 8;

        // Framing version.
        const version = view.getUint16(offset);
        offset += 2;
        if (version !== FRAMING_VERSION) {
            throw new CapsuleError(
                `unsupported framing version ${version}, expected ${FRAMING_VERSION}`,
            );
        }

        // Manifest length.
        const manifestLen = view.getUint32(offset);
        offset += 4;

        if (manifestLen > MAX_MANIFEST_BYTES) {
            throw new CapsuleError(
                `manifest length ${manifestLen} exceeds limit of ${MAX_MANIFEST_BYTES}`,
            );
        }
        if (offset + manifestLen > bytes.length) {
            throw new CapsuleError(`manifest length ${manifestLen} exceeds remaining data`);
        }

        // Manifest JSON.
        const manifestSlice = bytes.subarray(offset, offset + manifestLen);
        offset += manifestLen;

        let manifestJson: string;
        try {
            manifestJson = new TextDecoder('utf-8', { fatal: true }).decode(manifestSlice);
        } catch {
            throw new CapsuleError('manifest is not valid UTF-8');
        }

        let manifest: CapsuleManifest;
        try {
            manifest = JSON.parse(manifestJson);
        } catch {
            throw new CapsuleError('manifest is not valid JSON');
        }

        // Validate manifest structure.
        if (typeof manifest !== 'object' || manifest === null) {
            throw new CapsuleError('manifest must be an object');
        }
        if (manifest.schema_version !== MANIFEST_SCHEMA_VERSION) {
            throw new CapsuleError(
                `unsupported manifest schema version ${manifest.schema_version}, expected ${MANIFEST_SCHEMA_VERSION}`,
            );
        }
        if (!Array.isArray(manifest.items)) {
            throw new CapsuleError('manifest.items must be an array');
        }
        if (manifest.items.length === 0) {
            throw new CapsuleError('manifest.items must not be empty');
        }
        if (manifest.items.length > MAX_ITEMS) {
            throw new CapsuleError(
                `manifest has ${manifest.items.length} items, maximum is ${MAX_ITEMS}`,
            );
        }
        if (
            manifest.title !== null &&
            manifest.title !== undefined &&
            typeof manifest.title !== 'string'
        ) {
            throw new CapsuleError('manifest.title must be a string or null');
        }

        // Match declared sizes against remaining bytes.
        const items: CapsuleItem[] = [];
        for (const mi of manifest.items) {
            if (typeof mi.id !== 'string') throw new CapsuleError('item.id must be a string');
            if (typeof mi.name !== 'string') throw new CapsuleError('item.name must be a string');
            if (typeof mi.size !== 'number' || mi.size < 0 || !Number.isSafeInteger(mi.size)) {
                throw new CapsuleError(`item ${mi.id}: size must be a non-negative safe integer`);
            }
            if (typeof mi.media_type !== 'string')
                throw new CapsuleError(`item ${mi.id}: media_type must be a string`);
            if (typeof mi.presentation !== 'string')
                throw new CapsuleError(`item ${mi.id}: presentation must be a string`);
            if (!PRESENTATIONS.has(mi.presentation)) {
                throw new CapsuleError(
                    `item ${mi.id}: unknown presentation ${JSON.stringify(mi.presentation)}`,
                );
            }

            // Validate name length.
            if (new TextEncoder().encode(mi.name).length > MAX_NAME_BYTES) {
                throw new CapsuleError(`item ${mi.id}: name exceeds ${MAX_NAME_BYTES} UTF-8 bytes`);
            }

            items.push({
                id: mi.id,
                name: mi.name,
                size: mi.size,
                mediaType: mi.media_type,
                presentation: mi.presentation,
                ...(typeof mi.language === 'string' && mi.language.length > 0
                    ? { language: mi.language }
                    : {}),
                bytes: new Uint8Array(0), // filled in below
            });
        }

        // Verify item ids are unique.
        const seen = new Set<string>();
        for (const item of items) {
            if (seen.has(item.id)) {
                throw new CapsuleError(`duplicate item id ${item.id}`);
            }
            seen.add(item.id);
        }

        // Verify total item sizes against the remaining bytes. What follows the
        // last item is padding and must be zero bytes - nothing else may hide
        // there.
        let totalItemBytes = 0;
        for (const item of items) {
            totalItemBytes += item.size;
        }
        const remaining = bytes.length - offset;
        if (totalItemBytes > remaining) {
            throw new CapsuleError(
                `declared item sizes sum to ${totalItemBytes} bytes, but only ${remaining} bytes remain after the manifest`,
            );
        }
        for (let i = offset + totalItemBytes; i < bytes.length; i++) {
            if (bytes[i] !== 0) {
                throw new CapsuleError('padding after the last item contains non-zero bytes');
            }
        }

        // Slice item bytes.
        for (const item of items) {
            item.bytes = bytes.subarray(offset, offset + item.size);
            offset += item.size;
        }

        return new Capsule(items, manifest.title ?? null);
    }

    /**
     * Encodes this capsule into its binary representation, padded with zero
     * bytes to {@link padTo} its length so that the ciphertext size reveals
     * little more than the order of magnitude.
     */
    public encode(): Uint8Array {
        let totalSize = HEADER_BYTES + this.#manifestBytes.length;
        for (const item of this.items) {
            totalSize += item.bytes.length;
        }

        // Zero-initialised: the padding needs no explicit write.
        const out = new Uint8Array(padTo(totalSize));
        const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
        let offset = 0;

        // Magic.
        out.set(MAGIC, offset);
        offset += 8;

        // Framing version (u16, big-endian).
        view.setUint16(offset, FRAMING_VERSION);
        offset += 2;

        // Manifest length (u32, big-endian).
        view.setUint32(offset, this.#manifestBytes.length);
        offset += 4;

        // Manifest.
        out.set(this.#manifestBytes, offset);
        offset += this.#manifestBytes.length;

        // Items.
        for (const item of this.items) {
            out.set(item.bytes, offset);
            offset += item.bytes.length;
        }

        return out;
    }
}
