/** Encodes bytes as base64url without padding. */
export function toBase64url(bytes: Uint8Array): string {
    let binary = '';
    for (let i = 0; i < bytes.length; i++) {
        binary += String.fromCharCode(bytes[i]!);
    }
    return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

/** Decodes base64url (with or without padding) to bytes. */
export function fromBase64url(text: string): Uint8Array<ArrayBuffer> {
    let base64 = text.replaceAll('-', '+').replaceAll('_', '/');
    while (base64.length % 4 !== 0) {
        base64 += '=';
    }
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
}

/** Encodes a UTF-8 string to bytes. */
export function encodeUtf8(text: string): Uint8Array<ArrayBuffer> {
    return new TextEncoder().encode(text) as Uint8Array<ArrayBuffer>;
}

/** Decodes UTF-8 bytes to a string. */
export function decodeUtf8(bytes: Uint8Array): string {
    return new TextDecoder().decode(bytes);
}

/** Concatenates any number of byte arrays into one. */
export function concatByteArrays(...args: Uint8Array[]): Uint8Array {
    const length = args.reduce((sum, a) => sum + a.length, 0);
    const out = new Uint8Array(length);
    let offset = 0;
    for (const a of args) {
        out.set(a, offset);
        offset += a.length;
    }
    return out;
}
