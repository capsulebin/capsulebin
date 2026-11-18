/**
 * XSS vectors for the Markdown→DOMPurify pipeline.
 *
 * Each vector is fed through `renderMarkdown()` and the output is checked
 * to ensure no executable HTML, event handler, iframe, script or style
 * survives.  This is a unit-level test that exercises the allow-listed
 * tags/attributes without needing a browser.
 *
 * @see docs/threat-model.md (XSS through Markdown or code)
 */

import { describe, it, expect } from 'vitest';
import { renderMarkdown } from '../src/render-markdown';
import { highlightCode } from '../src/render-code';

// ----- helpers -----------------------------------------------------------

function noExec(html: string): void {
    // Nothing that can run code or load external resources.
    expect(html).not.toContain('<script');
    expect(html).not.toContain('onerror=');
    expect(html).not.toContain('onload=');
    expect(html).not.toContain('onclick=');
    expect(html).not.toContain('onmouseover=');
    expect(html).not.toContain('onfocus=');
    expect(html).not.toContain('<iframe');
    expect(html).not.toContain('<object');
    expect(html).not.toContain('<embed');
    expect(html).not.toContain('<form');
    expect(html).not.toContain('<style>');
    expect(html).not.toContain('javascript:');
    expect(html).not.toContain('<link');
}

// ----- corpus ------------------------------------------------------------

describe('XSS prevention in Markdown rendering', () => {
    it('bare script tag', () => {
        const html = renderMarkdown('<script>alert(1)</script>');
        noExec(html);
        expect(html).not.toContain('alert(1)');
    });

    it('script tag via Markdown HTML block', () => {
        const html = renderMarkdown('text\n\n<script>alert(1)</script>\n\nmore');
        noExec(html);
    });

    it('event handler on a link', () => {
        const html = renderMarkdown('<a onclick="alert(1)" href="/">click</a>');
        noExec(html);
    });

    it('event handler on markdown-generated link via HTML injection', () => {
        const html = renderMarkdown('[click](javascript:alert(1))');
        noExec(html);
    });

    it('javascript: href in HTML link', () => {
        // DOMPurify strips the href because it's not a safe URL.
        const html = renderMarkdown('<a href="javascript:alert(1)">click</a>');
        noExec(html);
        expect(html).not.toContain('javascript:');
    });

    it('iframe element', () => {
        const html = renderMarkdown('<iframe src="https://evil.com"></iframe>');
        noExec(html);
    });

    it('object element', () => {
        const html = renderMarkdown('<object data="https://evil.com"></object>');
        noExec(html);
    });

    it('embed element', () => {
        const html = renderMarkdown('<embed src="https://evil.com">');
        noExec(html);
    });

    it('style element', () => {
        const html = renderMarkdown('<style>body { background: red; }</style>');
        noExec(html);
    });

    it('image with onerror handler in HTML', () => {
        const html = renderMarkdown('<img src=x onerror="alert(1)">');
        noExec(html);
    });

    it('img becomes link in Markdown', () => {
        const html = renderMarkdown('![alt](https://evil.com/track.png)');
        noExec(html);
        expect(html).toContain('rel="noopener noreferrer"');
        expect(html).not.toContain('<img');
    });

    it('data:text/html in image source', () => {
        const html = renderMarkdown('![x](data:text/html,<script>alert(1)</script>)');
        noExec(html);
    });

    it('svg element', () => {
        const html = renderMarkdown('<svg onload="alert(1)"></svg>');
        noExec(html);
    });

    it('math element', () => {
        const html = renderMarkdown(
            '<math><annotation-xml encoding="text/html"><script>alert(1)</script></annotation-xml></math>',
        );
        noExec(html);
    });

    it('base tag', () => {
        const html = renderMarkdown('<base href="https://evil.com/">');
        noExec(html);
        expect(html).not.toContain('<base');
    });

    it('meta refresh', () => {
        const html = renderMarkdown('<meta http-equiv="refresh" content="0;url=https://evil.com">');
        noExec(html);
        expect(html).not.toContain('<meta');
    });

    it('script in code block does not execute', () => {
        const html = renderMarkdown('```html\n<script>alert(1)</script>\n```');
        noExec(html);
        // Code block content is text, not executable.
        expect(html).toContain('&lt;script&gt;');
    });

    it('form element stripped', () => {
        const html = renderMarkdown('<form action="https://evil.com"><input name="a"></form>');
        noExec(html);
    });

    it('textarea element stripped', () => {
        const html = renderMarkdown('<textarea onfocus="alert(1)">text</textarea>');
        noExec(html);
        expect(html).not.toContain('<textarea');
    });

    it('deeply nested event handlers', () => {
        const html = renderMarkdown(
            '<div><span><a onclick="alert(1)"><b>click</b></a></span></div>',
        );
        noExec(html);
    });

    it('raw HTML link with target stripped (reverse-tabnabbing)', () => {
        const html = renderMarkdown('<a href="https://evil.example" target="_blank">click</a>');
        noExec(html);
        expect(html).not.toContain('target');
    });
});

describe('XSS prevention in code highlighting', () => {
    it('escapes HTML in highlighted code', () => {
        const html = highlightCode('<img src=x onerror=alert(1)>', 'html');
        // Tags must be escaped — no raw HTML elements survive.
        // hljs splits `<img>` into `&lt;<span class="hljs-name">img</span>...&gt;`
        // so the angle brackets are escaped; `img` as a raw element name does not appear.
        expect(html).not.toContain('<img');
        expect(html).toContain('&lt;');
        expect(html).toContain('&gt;');
    });

    it('no script tags in output', () => {
        const html = highlightCode('<script>alert(1)</script>', 'javascript');
        expect(html).not.toContain('<script');
        expect(html).toContain('&lt;');
    });

    it('no inline style attributes', () => {
        for (const lang of ['rust', 'python', 'javascript', 'json']) {
            const html = highlightCode('let x = 1;', lang);
            expect(html).not.toContain('style=');
        }
    });

    describe('form controls', () => {
        it.each([
            '<input type="text" value="Enter your password">',
            '<input type="password">',
            '<input type="submit" value="Log in">',
            '<input type="image" src="x">',
            '<input>',
            '<input type=" TEXT ">',
        ])('removes %s', (raw) => {
            expect(renderMarkdown(`before ${raw} after`)).not.toContain('<input');
        });

        it('keeps task-list checkboxes, disabled', () => {
            const html = renderMarkdown('- [x] done\n- [ ] open');
            const doc = new DOMParser().parseFromString(html, 'text/html');
            const boxes = [...doc.querySelectorAll('input')];
            expect(boxes).toHaveLength(2);
            for (const box of boxes) {
                expect(box.type).toBe('checkbox');
                expect(box.disabled).toBe(true);
            }
        });

        it('disables a raw checkbox too', () => {
            const html = renderMarkdown('<input type="checkbox">');
            const box = new DOMParser().parseFromString(html, 'text/html').querySelector('input');
            expect(box?.disabled).toBe(true);
        });
    });
});
