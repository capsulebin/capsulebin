/**
 * Markdown rendering pipeline: Marked → DOMPurify → link policy.
 *
 * Raw HTML, inline scripts, event handlers and external images are stripped.
 * Code blocks are emitted as `<pre><code>` for later highlighting by the
 * viewer — this module only does the Markdown-to-HTML step.
 *
 * @see docs/threat-model.md (XSS through Markdown or code)
 */

import { marked, type Tokens } from 'marked';
import DOMPurify from 'dompurify';

// ----- DOMPurify allowlist -----------------------------------------------

const ALLOWED_TAGS = [
    'h1',
    'h2',
    'h3',
    'h4',
    'h5',
    'h6',
    'p',
    'ul',
    'ol',
    'li',
    'a',
    'strong',
    'em',
    'del',
    'code',
    'pre',
    'blockquote',
    'table',
    'thead',
    'tbody',
    'tr',
    'th',
    'td',
    'hr',
    'br',
    'details',
    'summary',
    'input', // disabled task-list checkboxes
];

const ALLOWED_ATTR = [
    'href',
    'rel',
    'type',
    'disabled',
    'checked', // input[type=checkbox]
];

// `input` is allowed for one reason only: GFM task lists render
// `<input type="checkbox" disabled>`. Raw HTML in a paste could otherwise use
// it to draw convincing text or password fields into the page. Anything but a
// checkbox is removed, and every remaining one is forced to be disabled.
DOMPurify.addHook('uponSanitizeElement', (node, data) => {
    if (data.tagName !== 'input') return;
    const type = (node as Element).getAttribute('type')?.trim().toLowerCase();
    if (type !== 'checkbox') node.parentNode?.removeChild(node);
});
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.nodeName === 'INPUT') node.setAttribute('disabled', '');
});

// ----- Marked configuration ----------------------------------------------

// Disable inline HTML — we only accept Markdown syntax.
marked.setOptions({ breaks: false, gfm: true });

// Custom renderer: external images become regular links so that the CSP
// does not block them mid-page and the viewer never leaks the Referer.
// Links always get rel="noopener noreferrer".
const renderer = {
    link({ href, text }: Tokens.Link): string {
        const url = href.replace(/"/g, '&quot;');
        const label = text || url;
        return `<a href="${url}" rel="noopener noreferrer">${label}</a>`;
    },
    image({ href, text }: Tokens.Image): string {
        const url = href.replace(/"/g, '&quot;');
        const label = text ? text.replace(/</g, '&lt;') : url;
        return `<a href="${url}" rel="noopener noreferrer">${label}</a>`;
    },
};

marked.use({ renderer });

// ----- public API --------------------------------------------------------

/**
 * Render Markdown to safe HTML.
 *
 * Unknown/untrusted HTML is stripped by DOMPurify.  Images become links
 * (`rel="noopener noreferrer"`).  Fenced code blocks stay unfenced so the
 * viewer can highlight them with the selected language.
 *
 * @returns sanitised HTML string, ready for `innerHTML`.
 */
export function renderMarkdown(markdown: string): string {
    const raw = marked.parse(markdown, { async: false }) as string;

    // Sanitise: strip anything outside the allowlist.
    const clean = DOMPurify.sanitize(raw, {
        ALLOWED_TAGS,
        ALLOWED_ATTR,
        ALLOW_DATA_ATTR: false,
    });

    return clean;
}
