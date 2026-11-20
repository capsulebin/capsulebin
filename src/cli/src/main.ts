/**
 * Executable entry point, bundled to `dist/cli.js`. Kept apart from `cli.ts`
 * so tests can import `run()` without starting the CLI.
 */

import { run } from './cli';
import { ExitCode } from './exit';
import { processIo } from './io';
import { escapeDiagnostic } from './terminal';

run(process.argv.slice(2), processIo()).then(
    (code) => {
        process.exitCode = code;
    },
    (err: unknown) => {
        process.stderr.write(`capsulebin: ${escapeDiagnostic(String(err))}\n`);
        process.exitCode = ExitCode.Failure;
    },
);
