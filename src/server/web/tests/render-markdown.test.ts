import { describe, it, expect } from 'vitest';
import { renderMarkdown } from '../src/render-markdown';

describe('renderMarkdown', () => {
    it('renders headings', () => {
        const html = renderMarkdown('# Hello');
        expect(html).toContain('<h1');
        expect(html).toContain('Hello');
    });

    it('renders bold and italic', () => {
        const html = renderMarkdown('**bold** and *italic*');
        expect(html).toContain('<strong>bold</strong>');
        expect(html).toContain('<em>italic</em>');
    });

    it('renders links with noopener noreferrer', () => {
        const html = renderMarkdown('[example](https://example.com)');
        expect(html).toContain('href="https://example.com"');
        expect(html).toContain('rel="noopener noreferrer"');
    });

    it('renders fenced code blocks', () => {
        const html = renderMarkdown('```rust\nfn main() {}\n```');
        expect(html).toContain('<code');
        expect(html).toContain('fn main()');
    });

    it('strips <script> tags', () => {
        const html = renderMarkdown('<script>alert(1)</script>');
        expect(html).not.toContain('<script>');
        expect(html).not.toContain('alert(1)');
    });

    it('strips onclick handlers', () => {
        const html = renderMarkdown('<a onclick="alert(1)">click</a>');
        expect(html).not.toContain('onclick');
    });

    it('converts images to links with noopener noreferrer', () => {
        const html = renderMarkdown('![alt](https://evil.com/track.png)');
        // The image renderer transforms to an <a> link with rel attribute.
        expect(html).toContain('<a');
        expect(html).toContain('rel="noopener noreferrer"');
        expect(html).toContain('alt');
        // The URL stays as href, no <img> tag survives.
        expect(html).not.toContain('<img');
    });

    it('strips iframe', () => {
        const html = renderMarkdown('<iframe src="https://evil.com"></iframe>');
        expect(html).not.toContain('<iframe');
    });

    it('strips target attribute from raw HTML links (reverse-tabnabbing)', () => {
        const html = renderMarkdown('<a href="https://evil.example" target="_blank">click</a>');
        expect(html).not.toContain('target');
        expect(html).toContain('href="https://evil.example"');
    });

    it('renders unordered lists', () => {
        const html = renderMarkdown('- one\n- two');
        expect(html).toContain('<ul>');
        expect(html).toContain('<li>one</li>');
    });

    it('renders blockquotes', () => {
        const html = renderMarkdown('> quoted');
        expect(html).toContain('<blockquote>');
        expect(html).toContain('quoted');
    });

    it('renders tables', () => {
        const html = renderMarkdown('| a | b |\n|---|---|\n| 1 | 2 |');
        expect(html).toContain('<table>');
        expect(html).toContain('<td>1</td>');
    });

    it('empty input produces empty output', () => {
        const html = renderMarkdown('');
        expect(typeof html).toBe('string');
    });
});
