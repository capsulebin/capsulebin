/**
 * Syntax highlighting via highlight.js with an explicit language allowlist.
 *
 * Languages are registered individually; unknown languages fall back to
 * `plaintext`. Output is purely class-based (`.hljs-*`) — no inline styles,
 * so it works under `style-src 'self'` without `'unsafe-inline'`.
 *
 * @see ADR-006
 */

import hljs from 'highlight.js/lib/core';

// ----- language allowlist ------------------------------------------------

/** Allowed languages. Each one must be imported below. */
const ALLOWED = new Set([
    'plaintext',
    'bash',
    'json',
    'yaml',
    'ini',
    'rust',
    'typescript',
    'javascript',
    'python',
    'sql',
    'html',
    'css',
    'diff',
    'markdown',
]);

// Register only the languages we actually ship.
import bash from 'highlight.js/lib/languages/bash';
import json from 'highlight.js/lib/languages/json';
import yaml from 'highlight.js/lib/languages/yaml';
import ini from 'highlight.js/lib/languages/ini';
import rust from 'highlight.js/lib/languages/rust';
import typescript from 'highlight.js/lib/languages/typescript';
import javascript from 'highlight.js/lib/languages/javascript';
import python from 'highlight.js/lib/languages/python';
import sql from 'highlight.js/lib/languages/sql';
import xml from 'highlight.js/lib/languages/xml'; // covers html
import css from 'highlight.js/lib/languages/css';
import diff from 'highlight.js/lib/languages/diff';
import markdown from 'highlight.js/lib/languages/markdown';
import plaintext from 'highlight.js/lib/languages/plaintext';

hljs.registerLanguage('bash', bash);
hljs.registerLanguage('json', json);
hljs.registerLanguage('yaml', yaml);
hljs.registerLanguage('ini', ini);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('python', python);
hljs.registerLanguage('sql', sql);
hljs.registerLanguage('html', xml);
hljs.registerLanguage('css', css);
hljs.registerLanguage('diff', diff);
hljs.registerLanguage('markdown', markdown);
hljs.registerLanguage('plaintext', plaintext);

// ----- public API --------------------------------------------------------

/**
 * Highlight source code, returning HTML with `.hljs-*` class annotations.
 *
 * Unknown or unsupported languages silently fall back to `plaintext` so that
 * not-yet-allowlisted languages are never a denial-of-service vector and the
 * caller does not need to branch.
 */
export function highlightCode(code: string, language: string): string {
    const lang = ALLOWED.has(language) ? language : 'plaintext';
    const result = hljs.highlight(code, { language: lang });
    // hljs returns { value: string } — the highlighted HTML with class spans.
    return result.value;
}

/** Exposed for tests. */
export { ALLOWED };
