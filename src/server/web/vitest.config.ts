import { defineConfig } from 'vitest/config';

import { catalogueModule } from './scripts/catalogue.ts';

export default defineConfig({
    // Every catalogue, chosen by `<html lang>`, so a test can switch the page
    // language; a production build has one language per build.
    plugins: [catalogueModule(null)],
    test: {
        // jsdom is enough and fast for the unit tests. Real browser behaviour (Web
        // Crypto, CSP, Trusted Types, rendering) is covered by the Playwright suite
        // in e2e/ (playwright.config.ts), which runs against the release binary.
        environment: 'jsdom',
        include: ['tests/**/*.test.ts'],
    },
});
