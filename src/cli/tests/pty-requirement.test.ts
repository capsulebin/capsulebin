/**
 * The terminal tests fail, rather than skip, when python3 is not on PATH, and
 * the failure names python3.
 */

import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { requirePython } from './pty';

describe.skipIf(process.platform === 'win32')('python3 requirement of the terminal tests', () => {
    const path = process.env['PATH'];
    let dir: string | undefined;

    afterEach(async () => {
        process.env['PATH'] = path;
        if (dir !== undefined) await rm(dir, { recursive: true, force: true });
        dir = undefined;
    });

    it('fails with a message naming python3 when python3 is not on PATH', async () => {
        dir = await mkdtemp(join(tmpdir(), 'capsulebin-no-python-'));
        process.env['PATH'] = dir;
        expect(() => requirePython()).toThrow(/python3/);
    });

    it('passes with python3 on PATH', () => {
        expect(() => requirePython()).not.toThrow();
    });
});
