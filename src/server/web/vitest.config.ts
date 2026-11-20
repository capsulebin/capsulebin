import { defineConfig } from 'vitest/config';

/**
 * These files regenerate `.shells/` (a production build, the dev server), so they
 * must not run at the same time: one would delete the shells under the other.
 */
const SHELL_GENERATING = ['tests/catalogue-modules.test.ts', 'tests/dev-server.acceptance.test.ts'];

export default defineConfig({
    test: {
        // jsdom is enough and fast for the unit tests. Real browser behaviour (Web
        // Crypto, CSP, Trusted Types, rendering) is covered by the Playwright suite
        // in e2e/ (playwright.config.ts), which runs against the release binary.
        environment: 'jsdom',
        projects: [
            {
                extends: true,
                test: {
                    name: 'unit',
                    include: ['tests/**/*.test.ts'],
                    exclude: SHELL_GENERATING,
                },
            },
            {
                extends: true,
                test: {
                    name: 'build',
                    include: SHELL_GENERATING,
                    // One file after the other, after the unit tests.
                    fileParallelism: false,
                    sequence: { groupOrder: 1 },
                },
            },
        ],
    },
});
