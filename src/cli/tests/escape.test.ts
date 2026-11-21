/**
 * The escaping functions of `terminal.ts` over every code point in
 * U+0000–U+00FF and over random strings: no control character ever comes
 * out, and text without control characters comes out unchanged.
 */

import { describe, expect, it } from 'vitest';
import { escapeDiagnostic, jsonLine } from '../src/terminal';

/* eslint-disable no-control-regex */
const CONTROLS = /[\u0000-\u001f\u007f-\u009f]/;
const RUNS = 2000;

/** A small seeded generator (mulberry32), so a failure can be reproduced. */
function random(seed: number): () => number {
    return () => {
        seed = (seed + 0x6d2b79f5) | 0;
        let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

/** Code points biased towards controls, CR/LF and multi-byte characters. */
const POOL = [
    ...Array.from({ length: 0x100 }, (_, i) => i),
    0x0a,
    0x0d,
    0x9b,
    0x1b,
    0x202e,
    0x200b,
    0xfeff,
    0xfffd,
    0x1f600,
    0x10ffff,
];

function randomString(next: () => number): string {
    const length = Math.floor(next() * 24);
    let text = '';
    for (let i = 0; i < length; i++) {
        text += String.fromCodePoint(POOL[Math.floor(next() * POOL.length)]!);
    }
    return text;
}

const without = (text: string, pattern: RegExp) => text.replace(new RegExp(pattern, 'g'), '');

describe('escapeDiagnostic', () => {
    it('escapes every code point in U+0000–U+00FF that is a control, and only those', () => {
        for (let code = 0; code <= 0xff; code++) {
            const char = String.fromCharCode(code);
            const escaped = escapeDiagnostic(`a${char}b`);
            expect(escaped).not.toMatch(CONTROLS);
            if (!CONTROLS.test(char)) expect(escaped).toBe(`a${char}b`);
        }
    });

    it('never emits a control character and keeps text without them', () => {
        const next = random(1);
        for (let run = 0; run < RUNS; run++) {
            const text = randomString(next);
            expect(escapeDiagnostic(text)).not.toMatch(CONTROLS);
            const clean = without(text, CONTROLS);
            expect(escapeDiagnostic(clean)).toBe(clean);
        }
    });
});

describe('jsonLine', () => {
    it('writes no DEL or C1 character and parses back to the same value', () => {
        const next = random(4);
        const strings = [
            Array.from({ length: 0x100 }, (_, i) => String.fromCharCode(i)).join(''),
            ...Array.from({ length: RUNS }, () => randomString(next)),
        ];
        for (const text of strings) {
            const line = jsonLine({ text });
            expect(line.endsWith('\n')).toBe(true);
            expect(line.slice(0, -1)).not.toMatch(CONTROLS);
            expect(JSON.parse(line)).toEqual({ text });
        }
    });
});
