/**
 * Trusted Types gate for the app's few `innerHTML` sinks.
 *
 * The CSP sends `require-trusted-types-for 'script'`, so Chromium-based
 * browsers throw on any `innerHTML = someString` unless the string was
 * produced by a registered Trusted Types policy. This module is that policy:
 * `createHTML` does not sanitize anything itself, it only brands a string as
 * safe for the DOM. Every call site is responsible for having already run
 * the value through {@link renderMarkdown} or {@link highlightCode} — see
 * `tests/trusted-html.test.ts` for the check that no `innerHTML` assignment
 * in `src/` bypasses this module.
 *
 * Firefox and Safari do not implement Trusted Types at all, so
 * `window.trustedTypes` is `undefined` there; the CSP directive is simply
 * unsupported and ignored by those browsers, and a plain string assigned to
 * `innerHTML` works as it always has.
 */

interface TrustedTypePolicy {
    createHTML(input: string): unknown;
}

interface TrustedTypePolicyFactory {
    createPolicy(name: string, rules: { createHTML(input: string): string }): TrustedTypePolicy;
}

declare global {
    interface Window {
        trustedTypes?: TrustedTypePolicyFactory;
    }
}

const policy: TrustedTypePolicy | null =
    typeof window !== 'undefined' && window.trustedTypes
        ? window.trustedTypes.createPolicy('capsulebin-html', {
              createHTML: (html: string) => html,
          })
        : null;

/**
 * Brands an already-sanitized HTML string for assignment to `innerHTML`.
 *
 * The return type stays `string` because the standard DOM typings do not
 * know about `TrustedHTML`; the browser accepts the branded object at
 * runtime regardless of what TypeScript believes its type to be.
 */
export function trustedHtml(html: string): string {
    return (policy ? policy.createHTML(html) : html) as string;
}
