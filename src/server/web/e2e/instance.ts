import { type ChildProcess, execFileSync, spawn } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * A second instance of the release binary, for tests that need a configuration
 * other than the one of playwright.config.ts. It gets a free port, its own
 * database and `public` creation mode; `env` adds or overrides settings.
 */

const BINARY = resolve(
    fileURLToPath(new URL('..', import.meta.url)),
    process.env.CAPSULEBIN_BINARY ?? '../../../target/release/capsulebin',
);

function freePort(): Promise<number> {
    return new Promise((done, failed) => {
        const probe = createServer();
        probe.on('error', failed);
        probe.listen(0, '127.0.0.1', () => {
            const address = probe.address();
            const port = typeof address === 'object' && address ? address.port : 0;
            probe.close(() => done(port));
        });
    });
}

async function ready(origin: string, server: ChildProcess): Promise<void> {
    for (let attempt = 0; attempt < 200; attempt++) {
        if (server.exitCode !== null) throw new Error(`the server exited with ${server.exitCode}`);
        try {
            if ((await fetch(`${origin}/health/ready`)).ok) return;
        } catch {
            /* Not listening yet. */
        }
        await new Promise((wait) => setTimeout(wait, 100));
    }
    throw new Error(`${origin} did not become ready`);
}

/**
 * A new creation token and the `creation.token_hashes` entry for it, from
 * `capsulebin token` as an operator gets them.
 */
export function creationToken(): { token: string; hash: string } {
    const output = execFileSync(BINARY, ['token'], { encoding: 'utf8' });
    const token = output.split('\n')[1]?.trim() ?? '';
    const hash = /sha256:[A-Za-z0-9_-]+/.exec(output)?.[0] ?? '';
    if (!token || !hash) throw new Error('could not read the output of capsulebin token');
    return { token, hash };
}

/** Runs `use` with the origin of a fresh instance, and stops the instance afterwards. */
export async function withInstance(
    env: Record<string, string>,
    use: (origin: string) => Promise<void>,
): Promise<void> {
    const port = await freePort();
    const origin = `http://127.0.0.1:${port}`;
    const data = mkdtempSync(join(tmpdir(), 'capsulebin-e2e-instance-'));
    const server = spawn(BINARY, ['serve'], {
        stdio: ['ignore', 'ignore', 'inherit'],
        env: {
            ...process.env,
            CAPSULEBIN__SERVER__BIND: `127.0.0.1:${port}`,
            CAPSULEBIN__SERVER__PUBLIC_URL: origin,
            CAPSULEBIN__DATABASE__PATH: join(data, 'capsulebin.db'),
            CAPSULEBIN__CREATION__MODE: 'public',
            CAPSULEBIN_LOG: 'warn',
            ...env,
        },
    });
    try {
        await ready(origin, server);
        await use(origin);
    } finally {
        server.kill();
    }
}
