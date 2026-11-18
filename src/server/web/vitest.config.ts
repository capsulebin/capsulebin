import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        // The client is plain DOM without a framework, so jsdom is enough and fast.
        // Real browser behaviour (Web Crypto, CSP, rendering) will be covered by a
        // separate Playwright suite.
        environment: 'jsdom',
        include: ['tests/**/*.test.ts'],
    },
});
