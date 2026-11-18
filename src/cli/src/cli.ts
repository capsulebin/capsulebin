/**
 * CapsuleBin CLI: `put`, `get` and `delete` against a CapsuleBin server.
 *
 * Built on @capsulebin/core, the same code the web client runs, so a capsule
 * created here opens in the browser and vice versa. Encryption and decryption
 * happen in this process; the server only ever sees ciphertext (ADR-001).
 *
 * Exit codes are listed in `exit.ts` and in `capsulebin --help`.
 */

import { parseArgs, type ParseArgsConfig } from 'node:util';
import { put } from './commands/put';
import { get } from './commands/get';
import { remove } from './commands/delete';
import { ExitCode, toCliError, usage } from './exit';
import type { Io } from './io';
import { AS_VALUES } from './items';

import pkg from '../package.json' with { type: 'json' };

const VERSION = pkg.version;

export const HELP = `capsulebin — share end-to-end encrypted capsules

Usage:
  capsulebin put [FILE...] [options]     Encrypt files (or stdin) into one capsule and upload it
  capsulebin get LINK [options]          Retrieve and decrypt a capsule from its share link
  capsulebin delete LINK [options]       Delete a capsule with its management link

put options:
  -s, --server URL         Server base URL (or CAPSULEBIN_SERVER)
  -e, --expires DURATION   e.g. 30m, 12h, 7d, 2w, or "never"; default: the server's default
  -b, --burn               Delete the capsule after its first retrieval
  -p, --password           Protect with a password (CAPSULEBIN_PASSWORD, or prompt)
      --password-file PATH Read the password from the first line of PATH
      --token-file PATH    Creation token for servers that require one (or CAPSULEBIN_TOKEN)
      --as KIND            text | code | markdown | file (default: inferred per input)
      --lang LANG          Highlighting language for code (implies --as code)
      --name NAME          Item name for stdin or a single input
      --title TITLE        Capsule title
      --retries N          Retries on network errors, 429 and 5xx (default 3)

get options:
  -o, --output DIR         Write all items to DIR (default: a single item goes to stdout,
                           several items to the current directory)
      --password-file PATH Read the password from the first line of PATH
                           (or CAPSULEBIN_PASSWORD, or prompt)
  -y, --yes                Retrieve a burn-after-retrieval capsule without asking
  -f, --force              Overwrite existing files instead of picking a free name

Common options:
      --json               Machine-readable output on stdout
  -h, --help               Show this help
  -V, --version            Show the version

Secrets are never accepted as arguments: passwords and tokens come from files,
environment variables or an interactive prompt.

Exit codes:
  0  success                 5  could not decrypt / malformed capsule
  1  unexpected error        6  rejected by the server (token, size, expiry, rate limit)
  2  usage error             7  server unreachable or server error
  3  capsule not available   8  needs a password or burn confirmation
  4  burn capsule retrieved concurrently by someone else
`;

const OPTIONS = {
    server: { type: 'string', short: 's' },
    expires: { type: 'string', short: 'e' },
    burn: { type: 'boolean', short: 'b' },
    password: { type: 'boolean', short: 'p' },
    'password-file': { type: 'string' },
    'token-file': { type: 'string' },
    as: { type: 'string' },
    lang: { type: 'string' },
    name: { type: 'string' },
    title: { type: 'string' },
    retries: { type: 'string' },
    output: { type: 'string', short: 'o' },
    yes: { type: 'boolean', short: 'y' },
    force: { type: 'boolean', short: 'f' },
    json: { type: 'boolean' },
    help: { type: 'boolean', short: 'h' },
    version: { type: 'boolean', short: 'V' },
} satisfies ParseArgsConfig['options'];

type Values = ReturnType<
    typeof parseArgs<{ options: typeof OPTIONS; allowPositionals: true }>
>['values'];

const COMMAND_OPTIONS: Record<string, (keyof typeof OPTIONS)[]> = {
    put: [
        'server',
        'expires',
        'burn',
        'password',
        'password-file',
        'token-file',
        'as',
        'lang',
        'name',
        'title',
        'retries',
    ],
    get: ['output', 'password-file', 'yes', 'force'],
    delete: [],
};
const COMMON_OPTIONS: (keyof typeof OPTIONS)[] = ['json', 'help', 'version'];

const UNITS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86_400, w: 604_800 };

/** `30m`, `12h`, `7d`, `2w`, `90s` → seconds; `never` → `null`. */
export function parseDuration(text: string): number | null {
    if (text === 'never') return null;
    const match = /^(\d+)([smhdw])$/.exec(text);
    if (!match) throw usage(`invalid duration "${text}" (examples: 30m, 12h, 7d, never)`);
    const seconds = Number(match[1]) * UNITS[match[2]!]!;
    if (seconds <= 0 || !Number.isSafeInteger(seconds)) {
        throw usage(`invalid duration "${text}"`);
    }
    return seconds;
}

function parseRetries(text: string | undefined): number {
    if (text === undefined) return 3;
    const n = Number(text);
    if (!Number.isInteger(n) || n < 0 || n > 10) throw usage('--retries must be 0–10');
    return n;
}

function parse(argv: string[]): {
    command: string | undefined;
    positionals: string[];
    values: Values;
} {
    let parsed;
    try {
        parsed = parseArgs({ args: argv, options: OPTIONS, allowPositionals: true, strict: true });
    } catch (err) {
        throw usage(err instanceof Error ? err.message : String(err));
    }
    const [command, ...positionals] = parsed.positionals;
    return { command, positionals, values: parsed.values };
}

function checkOptions(command: string, values: Values): void {
    const allowed = new Set([...(COMMAND_OPTIONS[command] ?? []), ...COMMON_OPTIONS]);
    for (const key of Object.keys(values)) {
        if (!allowed.has(key as keyof typeof OPTIONS)) {
            throw usage(`--${key} does not apply to "${command}"`);
        }
    }
}

/** Runs the CLI and returns the exit code. Never calls `process.exit`. */
export async function run(argv: string[], io: Io): Promise<number> {
    let json = argv.includes('--json');
    try {
        const { command, positionals, values } = parse(argv);
        json = values.json === true;

        if (values.version) {
            io.stdout.write(`capsulebin ${VERSION}\n`);
            return ExitCode.Ok;
        }
        if (values.help || command === 'help' || command === undefined) {
            (command === undefined && !values.help ? io.stderr : io.stdout).write(HELP);
            return command === undefined && !values.help ? ExitCode.Usage : ExitCode.Ok;
        }
        if (!(command in COMMAND_OPTIONS)) {
            throw usage(`unknown command "${command}" (expected put, get or delete)`);
        }
        checkOptions(command, values);

        switch (command) {
            case 'put': {
                const server = (values.server ?? io.env['CAPSULEBIN_SERVER'] ?? '').replace(
                    /\/+$/,
                    '',
                );
                if (!server) throw usage('no server: pass --server URL or set CAPSULEBIN_SERVER');
                if (!/^https?:\/\//.test(server)) throw usage('--server must be an http(s) URL');

                let as;
                if (values.as !== undefined) {
                    as = AS_VALUES[values.as];
                    if (!as) throw usage(`--as must be one of text, code, markdown, file`);
                }
                if (values.lang !== undefined && as !== undefined && as !== 'code') {
                    throw usage('--lang only applies to code');
                }
                if (values.password && values['password-file']) {
                    throw usage('use either --password or --password-file');
                }

                await put(io, {
                    inputs: positionals,
                    server,
                    expiresIn:
                        values.expires === undefined ? undefined : parseDuration(values.expires),
                    burn: values.burn === true,
                    password: values.password === true,
                    passwordFile: values['password-file'],
                    tokenFile: values['token-file'],
                    as,
                    lang: values.lang,
                    name: values.name,
                    title: values.title,
                    retries: parseRetries(values.retries),
                    json,
                });
                return ExitCode.Ok;
            }
            case 'get':
            case 'delete': {
                if (positionals.length !== 1) throw usage(`${command} expects exactly one link`);
                const link = positionals[0]!;
                if (command === 'get') {
                    await get(io, {
                        link,
                        output: values.output,
                        passwordFile: values['password-file'],
                        yes: values.yes === true,
                        force: values.force === true,
                        json,
                    });
                } else {
                    await remove(io, { link, json });
                }
                return ExitCode.Ok;
            }
        }
        return ExitCode.Usage;
    } catch (err) {
        const error = toCliError(err);
        if (json) {
            io.stdout.write(
                JSON.stringify({ error: { reason: error.reason, message: error.message } }) + '\n',
            );
        }
        io.stderr.write(`capsulebin: ${error.message}\n`);
        if (error.exitCode === ExitCode.Usage && !json) {
            io.stderr.write('Run "capsulebin --help" for usage.\n');
        }
        return error.exitCode;
    }
}
