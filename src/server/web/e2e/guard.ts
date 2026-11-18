import { test as base, expect, type BrowserContext, type Page } from '@playwright/test';

declare global {
    interface Window {
        __capsulebinViolation?: (directive: string, detail: string) => void;
        __capsulebinXss?: (where: string) => void;
    }
}

export interface Guard {
    /** Everything recorded so far that is not covered by {@link Guard.allow}. */
    readonly problems: string[];
    /**
     * Accepts CSP violations of `directive` whose source matches `source`, for
     * this test only. Meant for reports an engine raises about content it blocks
     * on purpose; every use needs a comment saying why.
     */
    allow(directive: string, source: RegExp): void;
}

/**
 * Records everything in a browser context that must never happen: CSP and
 * Trusted Types violations, uncaught page errors, console errors, and the XSS
 * canary `__capsulebinXss()` being called from injected markup.
 *
 * The reports go through bindings rather than page state, so they survive the
 * page being closed or navigated before the test looks at them.
 */
export async function installGuard(context: BrowserContext): Promise<Guard> {
    const recorded: { directive?: string; source?: string; message: string }[] = [];
    const allowed: { directive: string; source: RegExp }[] = [];

    await context.exposeBinding(
        '__capsulebinViolation',
        (_page, directive: string, detail: string) => {
            recorded.push({
                directive,
                source: detail,
                message: `violation: ${directive} ${detail}`,
            });
        },
    );
    await context.exposeBinding('__capsulebinXss', (_page, where: string) => {
        recorded.push({ message: `XSS canary executed: ${where}` });
    });
    await context.addInitScript(() => {
        document.addEventListener('securitypolicyviolation', (event) => {
            window.__capsulebinViolation?.(
                event.violatedDirective,
                `blocked=${event.blockedURI} at ${event.sourceFile}:${event.lineNumber}`,
            );
        });
    });

    const watch = (page: Page) => {
        page.on('pageerror', (error) => recorded.push({ message: `page error: ${error.message}` }));
        page.on('console', (message) => {
            const text = message.text();
            // The server answers unknown, consumed and wrong-password capsules
            // with 404, and too many wrong passwords with 429; engines log both
            // as a failed resource load. CSP violations arrive through the event
            // above with their source, so their console echo is not counted twice.
            if (message.type() !== 'error' || /\b(404|429)\b/.test(text)) return;
            if (/Content Security Policy|Trusted Type/i.test(text)) return;
            recorded.push({ message: `console error: ${text}` });
        });
    };
    context.pages().forEach(watch);
    context.on('page', watch);

    return {
        get problems() {
            return recorded
                .filter(
                    (r) =>
                        !allowed.some(
                            (a) =>
                                r.directive === a.directive &&
                                r.source !== undefined &&
                                a.source.test(r.source),
                        ),
                )
                .map((r) => r.message);
        },
        allow(directive, source) {
            allowed.push({ directive, source });
        },
    };
}

/** `test` with the guard installed in every test's context; any record fails the test. */
export const test = base.extend<{ guard: Guard }>({
    guard: [
        async ({ context }, use) => {
            const guard = await installGuard(context);
            await use(guard);
            expect(guard.problems, 'the browser reported problems').toEqual([]);
        },
        { auto: true },
    ],
});

export { expect };
