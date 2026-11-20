/**
 * Keeps sender- and server-controlled control characters away from the
 * terminal: a terminal interprets them (colours, cursor moves, a window title,
 * clearing the screen, overwriting a line), a pipe or a file does not.
 *
 * A control character is shown as a visible escape instead: C0 and DEL as
 * `\xNN`, C1 (U+0080–U+009F) as `\u00NN`, and a byte that is not valid UTF-8
 * as `\xNN`, so U+009B and the byte 0x9B stay distinguishable. A backslash is
 * left alone: the escapes are for reading, not for decoding back. Bidi and
 * invisible characters are not terminal controls and pass unchanged.
 */

/* eslint-disable no-control-regex */

/** C0 except `\t`, `\n`, and `\r` before `\n`; DEL; C1. */
const CONTENT_CONTROLS = /\r(?!\n)|[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g;
/** Every C0 character (`\n` and `\t` too), DEL and C1. */
const DIAGNOSTIC_CONTROLS = /[\u0000-\u001f\u007f-\u009f]/g;
/** DEL and C1, which `JSON.stringify` leaves raw. */
const JSON_CONTROLS = /[\u007f-\u009f]/g;

function hex(code: number): string {
    return code.toString(16).padStart(2, '0');
}

function escapeControl(char: string): string {
    const code = char.charCodeAt(0);
    return code < 0x80 ? `\\x${hex(code)}` : `\\u00${hex(code)}`;
}

/**
 * Item content for a terminal: control characters and invalid UTF-8 bytes as
 * escapes; `\n`, `\t`, `\r\n` and printable text unchanged.
 */
export function escapeContent(bytes: Uint8Array): string {
    const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
    const parts: string[] = [];
    let start = 0;
    let i = 0;
    while (i < bytes.length) {
        const length = sequenceLength(bytes, i);
        if (length > 0) {
            i += length;
            continue;
        }
        parts.push(
            decoder.decode(bytes.subarray(start, i)).replace(CONTENT_CONTROLS, escapeControl),
        );
        parts.push(`\\x${hex(bytes[i]!)}`);
        i += 1;
        start = i;
    }
    parts.push(decoder.decode(bytes.subarray(start)).replace(CONTENT_CONTROLS, escapeControl));
    return parts.join('');
}

/**
 * Text for a diagnostic line on stderr (a server's problem title, a file
 * name, …): every control character as an escape, line breaks included, so
 * the text cannot start a line of its own.
 */
export function escapeDiagnostic(text: string): string {
    return text.replace(DIAGNOSTIC_CONTROLS, escapeControl);
}

/**
 * `value` as one line of JSON. DEL and C1 are written as `\u00NN` (the parsed
 * value is the same), so no control character reaches a terminal showing it.
 */
export function jsonLine(value: unknown): string {
    const escape = (char: string) => `\\u00${hex(char.charCodeAt(0))}`;
    return JSON.stringify(value).replace(JSON_CONTROLS, escape) + '\n';
}

/**
 * The length of the well-formed UTF-8 sequence at `bytes[i]` (Unicode table
 * 3-7: no overlong forms, surrogates or code points above U+10FFFF), or 0.
 */
function sequenceLength(bytes: Uint8Array, i: number): number {
    const lead = bytes[i]!;
    if (lead < 0x80) return 1;
    let length;
    let low = 0x80;
    let high = 0xbf;
    if (lead >= 0xc2 && lead <= 0xdf) {
        length = 2;
    } else if (lead >= 0xe0 && lead <= 0xef) {
        length = 3;
        if (lead === 0xe0) low = 0xa0;
        if (lead === 0xed) high = 0x9f;
    } else if (lead >= 0xf0 && lead <= 0xf4) {
        length = 4;
        if (lead === 0xf0) low = 0x90;
        if (lead === 0xf4) high = 0x8f;
    } else {
        return 0;
    }
    for (let k = 1; k < length; k++) {
        const next = bytes[i + k];
        if (next === undefined || next < low || next > high) return 0;
        low = 0x80;
        high = 0xbf;
    }
    return length;
}
