/**
 * Executable entry point, bundled to `dist/cli.js`. Kept apart from `cli.ts`
 * so tests can import `run()` without starting the CLI.
 */

import { run } from './cli';
import { ExitCode } from './exit';
import { processIo } from './io';

run(process.argv.slice(2), processIo()).then(
    (code) => {
        process.exitCode = code;
    },
    (err: unknown) => {
        process.stderr.write(`capsulebin: ${String(err)}\n`);
        process.exitCode = ExitCode.Failure;
    },
);
