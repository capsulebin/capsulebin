import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { defineConfig, devices } from '@playwright/test';

/**
 * Browser tests against the release binary, in Chromium, Firefox and WebKit.
 *
 * They exercise what no other test does: the shipped bundle running in a real
 * engine under the server's CSP and Trusted Types. Unit tests run in jsdom, which
 * enforces neither, and smoke.sh checks the headers but executes no script.
 *
 * The binary is started fresh for each run, with its assets embedded and an empty
 * database, in `public` creation mode so the create page works without a token.
 *
 *   npm run build -w @capsulebin/web
 *   cargo build --release --locked -p capsulebin-server
 *   npm run e2e -w @capsulebin/web                        # all three engines
 *   npm run e2e -w @capsulebin/web -- --project=chromium  # one engine
 *
 * CAPSULEBIN_BINARY points to another binary, CAPSULEBIN_E2E_PORT moves the port.
 */

const PORT = Number(process.env.CAPSULEBIN_E2E_PORT ?? 18090);
const BINARY = resolve(
    import.meta.dirname,
    process.env.CAPSULEBIN_BINARY ?? '../../../target/release/capsulebin',
);

// The config is loaded by the runner and again by every worker; the database
// directory is created once and handed down through the environment.
process.env.CAPSULEBIN_E2E_DATA ??= mkdtempSync(join(tmpdir(), 'capsulebin-e2e-'));

export default defineConfig({
    testDir: 'e2e',
    testMatch: '**/*.e2e.ts',
    fullyParallel: true,
    forbidOnly: Boolean(process.env.CI),
    reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
    timeout: 60_000,
    use: {
        baseURL: `http://127.0.0.1:${PORT}`,
        trace: 'retain-on-failure',
    },
    projects: [
        { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
        { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
        { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    ],
    webServer: {
        command: `"${BINARY}" serve`,
        url: `http://127.0.0.1:${PORT}/health/ready`,
        reuseExistingServer: false,
        env: {
            CAPSULEBIN__SERVER__BIND: `127.0.0.1:${PORT}`,
            CAPSULEBIN__SERVER__PUBLIC_URL: `http://127.0.0.1:${PORT}`,
            CAPSULEBIN__DATABASE__PATH: join(process.env.CAPSULEBIN_E2E_DATA, 'capsulebin.db'),
            CAPSULEBIN__CREATION__MODE: 'public',
            // Exercised by legal.e2e.ts; privacy notice and terms are the built-in ones.
            CAPSULEBIN__LEGAL__OPERATOR__TEXT: 'Example Operator GmbH  \nStreet 1, 12345 City',
            CAPSULEBIN__LEGAL__HOSTING__TEXT: 'Example Hosting AG',
            CAPSULEBIN_LOG: 'warn',
        },
    },
});
