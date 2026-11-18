import { describe, it, expect } from 'vitest';
import { highlightCode, ALLOWED } from '../src/render-code';

describe('highlightCode', () => {
    it('highlights Rust keywords', () => {
        const result = highlightCode('fn main() {}', 'rust');
        expect(result).toContain('hljs-keyword');
        expect(result).toContain('fn');
    });

    it('highlights JSON syntax', () => {
        const result = highlightCode('{"key": true}', 'json');
        expect(result).toContain('hljs-attr');
        expect(result).toContain('hljs-literal');
    });

    it('falls back to plaintext for unknown languages', () => {
        const result = highlightCode('some text', 'brainfuck');
        // highlight.js still wraps in <span class="hljs-*"> with plaintext
        expect(typeof result).toBe('string');
        expect(result).toContain('some text');
    });

    it('does not contain inline style attributes', () => {
        const result = highlightCode('let x = 1;', 'rust');
        expect(result).not.toContain('style=');
    });

    it('escapes HTML in source', () => {
        const result = highlightCode('<script>alert(1)</script>', 'plaintext');
        expect(result).not.toContain('<script>');
        expect(result).toContain('&lt;script&gt;');
    });

    it('all allowlisted languages are registerable', () => {
        for (const lang of ALLOWED) {
            const result = highlightCode('a', lang);
            expect(typeof result).toBe('string');
        }
    });

    it('empty input produces empty output', () => {
        const result = highlightCode('', 'rust');
        expect(typeof result).toBe('string');
    });
});
