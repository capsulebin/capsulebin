/**
 * Keeps sender- and server-controlled control characters away from the
 * terminal in diagnostics and `--json` output: a terminal interprets them
 * (colours, cursor moves, a window title, clearing the screen, overwriting a
 * line). Item content never reaches the terminal; `get` writes it to files.
 *
 * A control character is shown as a visible escape instead: C0 and DEL as
 * `\xNN`, C1 (U+0080–U+009F) as `\u00NN`. A backslash is left alone: the
 * escapes are for reading, not for decoding back. Bidi and invisible
 * characters are not terminal controls and pass unchanged.
 */

/* eslint-disable no-control-regex */

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
