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
import { hasQueryOrFragment, LinkError, trailingLanguage } from '@capsulebin/core';
import { put } from './commands/put';
import { get } from './commands/get';
import { remove } from './commands/delete';
import { CliError, ExitCode, toCliError, usage } from './exit';
import type { Io } from './io';
import { AS_VALUES } from './items';
import { readLink } from './secrets';
import { escapeDiagnostic, jsonLine } from './terminal';
import { allowHttp, checkTransport } from './transport';

import pkg from '../package.json' with { type: 'json' };

const VERSION = pkg.version;

export const HELP = `capsulebin — share end-to-end encrypted capsules

Usage:
  capsulebin put [FILE...] [options]     Encrypt files (or stdin) into one capsule and upload it
  capsulebin get LINK [options]          Retrieve and decrypt a capsule from its share link
  capsulebin delete LINK [options]       Delete a capsule with its management link

LINK is the link itself, or - to read it from the first line of stdin; or leave it
out and pass --link-file PATH. A link given as an argument is visible to other
local users (ps) and lands in your shell history; prefer - or --link-file.

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
      --delete-link-file PATH
                           Write the delete link to PATH, a new file with mode 0600,
                           instead of stderr or the JSON output

get options:
  -s, --server URL         The server you trust with CAPSULEBIN_PASSWORD (or CAPSULEBIN_SERVER):
                           the variable is only used for links to this server
  -o, --output DIR         Write the items to DIR, created if missing (default: a single
                           item goes to the current directory, several items to a new
                           directory capsule-DATE-ID in it); existing files are never
                           overwritten, an item gets a free name such as "name (1).txt"
      --here               Write several items directly to the current directory
      --password-file PATH Read the password from the first line of PATH (or
                           CAPSULEBIN_PASSWORD for links to --server, or prompt)
  -y, --yes                Retrieve a burn-after-retrieval capsule without asking
      --link-file PATH     Read the share link from the first line of PATH

delete options:
      --link-file PATH     Read the management link from the first line of PATH

Common options:
      --allow-http         Allow plain http:// to hosts other than localhost, 127.0.0.0/8
                           and [::1] (or CAPSULEBIN_ALLOW_HTTP=1); refused by default
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
    'delete-link-file': { type: 'string' },
    'link-file': { type: 'string' },
    output: { type: 'string', short: 'o' },
    here: { type: 'boolean' },
    yes: { type: 'boolean', short: 'y' },
    'allow-http': { type: 'boolean' },
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
        'delete-link-file',
    ],
    get: ['server', 'output', 'here', 'password-file', 'yes', 'link-file'],
    delete: ['link-file'],
};
const COMMON_OPTIONS: (keyof typeof OPTIONS)[] = ['allow-http', 'json', 'help', 'version'];

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

/**
 * Refuses a server URL that requests and links cannot be built on: not
 * http(s); with a query or fragment, which would swallow every path appended
 * to it; or ending in a language code, where `get` and `delete` would read
 * the code in the link as the language segment and contact the base without
 * it. Takes the URL as given, before any trailing slash is removed.
 */
function checkServer(server: string): void {
    let language;
    try {
        language = /^https?:\/\//.test(server) ? trailingLanguage(server) : undefined;
    } catch (err) {
        if (!(err instanceof LinkError)) throw err;
    }
    if (language === undefined) throw usage('--server must be an http(s) URL');
    if (hasQueryOrFragment(server)) {
        throw usage(
            '--server must not contain a query or fragment: the API path is appended to it',
        );
    }
    if (language !== null) {
        throw usage(
            `--server must not end in the language code "${language}": links below it would be ` +
                `ambiguous with the /${language}/ language prefix of the pages`,
        );
    }
}

/**
 * A usable server base: {@link checkServer}'s rules plus the HTTP policy.
 * Returns it without trailing slashes.
 */
function serverBase(given: string, httpAllowed: boolean): string {
    checkServer(given);
    checkTransport(given, httpAllowed, '--server');
    return given.replace(/\/+$/, '');
}

/**
 * The server `get` trusts with `CAPSULEBIN_PASSWORD`: `--server`, which must
 * be usable, else `CAPSULEBIN_SERVER` if usable. An unusable variable (it may
 * be set for `put`) is no anchor rather than an error, so it cannot break
 * `get` of a capsule without a password.
 */
function passwordAnchor(io: Io, given: string | undefined, httpAllowed: boolean) {
    if (given !== undefined) return serverBase(given, httpAllowed);
    try {
        return serverBase(io.env['CAPSULEBIN_SERVER'] ?? '', httpAllowed);
    } catch (err) {
        if (err instanceof CliError) return undefined;
        throw err;
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
        const httpAllowed = allowHttp(io, values['allow-http'] === true);

        switch (command) {
            case 'put': {
                const given = values.server ?? io.env['CAPSULEBIN_SERVER'] ?? '';
                if (!given.replace(/\/+$/, '')) {
                    throw usage('no server: pass --server URL or set CAPSULEBIN_SERVER');
                }
                const server = serverBase(given, httpAllowed);

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
                    deleteLinkFile: values['delete-link-file'],
                    json,
                });
                return ExitCode.Ok;
            }
            case 'get':
            case 'delete': {
                if (values.here && values.output !== undefined) {
                    throw usage('use either --here or --output');
                }
                const link = await readLink(io, command, positionals, values['link-file']);
                if (command === 'get') {
                    await get(io, {
                        link,
                        anchor: passwordAnchor(io, values.server, httpAllowed),
                        allowHttp: httpAllowed,
                        output: values.output,
                        here: values.here === true,
                        passwordFile: values['password-file'],
                        yes: values.yes === true,
                        json,
                    });
                } else {
                    await remove(io, { link, allowHttp: httpAllowed, json });
                }
                return ExitCode.Ok;
            }
        }
        return ExitCode.Usage;
    } catch (err) {
        const error = toCliError(err);
        if (json) {
            io.stdout.write(jsonLine({ error: { reason: error.reason, message: error.message } }));
        }
        io.stderr.write(`capsulebin: ${escapeDiagnostic(error.message)}\n`);
        if (error.exitCode === ExitCode.Usage && !json) {
            io.stderr.write('Run "capsulebin --help" for usage.\n');
        }
        return error.exitCode;
    }
}
