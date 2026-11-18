import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        // jsdom is enough and fast for the unit tests. Real browser behaviour (Web
        // Crypto, CSP, Trusted Types, rendering) is covered by the Playwright suite
        // in e2e/ (playwright.config.ts), which runs against the release binary.
        environment: 'jsdom',
        include: ['tests/**/*.test.ts'],
    },
});
