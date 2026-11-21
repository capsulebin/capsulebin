/**
 * Where passwords, creation tokens and links come from.
 *
 * Never a password or token from a command-line argument: argv is visible to
 * every user on the machine (`ps`) and lands in shell history. The sources are
 * a file, an environment variable, or an interactive prompt. A link may still
 * be an argument for compatibility, but can come from stdin or a file instead.
 */

import { createReadStream } from 'node:fs';
import { resolve } from 'node:path';
import { CliError, ExitCode, usage } from './exit';
import { type Io, readFirstLine } from './io';
import { escapeDiagnostic } from './terminal';
import { normalizeBase } from './transport';

/**
 * The most bytes the first line of a link, token or password source may hold.
 * Fixed: it bounds memory and ends a source without a line ending.
 */
const MAX_LINE_BYTES = 16_384;

/**
 * Text of a first line read from a source. A file keeps a leading byte order
 * mark (`ignoreBOM`), stdin drops it.
 */
function decodeLine(line: Uint8Array, keepBom: boolean): string {
    return new TextDecoder('utf-8', { ignoreBOM: keepBom }).decode(line);
}

/** A first line above {@link MAX_LINE_BYTES}. Names the source, never the content. */
function lineTooLong(what: string, source: string): CliError {
    return usage(`the first line of the ${what} from ${source} exceeds ${MAX_LINE_BYTES} bytes`);
}

/** First line of a file, without its line ending. Nothing beyond it is read. */
async function readSecretFile(io: Io, path: string, what: string): Promise<string> {
    let line: Uint8Array | null;
    try {
        line = await readFirstLine(createReadStream(resolve(io.cwd, path)), MAX_LINE_BYTES);
    } catch {
        throw usage(`cannot read ${what} file ${path}`);
    }
    if (line === null) throw lineTooLong(what, `file ${path}`);
    if (line.length === 0) throw usage(`${what} file ${path} is empty`);
    return decodeLine(line, true);
}

/**
 * The link for `get`/`delete`: exactly one of a positional link, `-` (the
 * first line of stdin) or `--link-file` (the first line of the file). No
 * message quotes what was read.
 */
export async function readLink(
    io: Io,
    command: string,
    positionals: string[],
    linkFile: string | undefined,
): Promise<string> {
    if (positionals.length > 1) throw usage(`${command} expects exactly one link`);
    const given = positionals[0];
    if (linkFile !== undefined) {
        if (given !== undefined) throw usage('give the link either as an argument or --link-file');
        return readSecretFile(io, linkFile, 'link');
    }
    if (given === undefined) {
        throw usage(`${command} expects a link: an argument, - for stdin, or --link-file PATH`);
    }
    if (given !== '-') return given;
    const line = await io.readStdinLine(MAX_LINE_BYTES);
    if (line === null) throw lineTooLong('link', 'stdin');
    const link = decodeLine(line, false);
    if (!link) throw usage('no link on stdin');
    return link;
}

/**
 * The creation token for `server`: `--token-file` (for any server), else
 * `CAPSULEBIN_TOKEN`, else none.
 *
 * With `CAPSULEBIN_SERVER` set, `CAPSULEBIN_TOKEN` is that server's token: it
 * is withheld, with a notice on stderr, when `server` is another one or the
 * variable is no URL to compare with. Without `CAPSULEBIN_SERVER` the token
 * goes to whichever server is in use.
 */
export async function creationToken(
    io: Io,
    tokenFile: string | undefined,
    server: string,
): Promise<string | undefined> {
    if (tokenFile !== undefined) return readSecretFile(io, tokenFile, 'token');
    const fromEnv = io.env['CAPSULEBIN_TOKEN'];
    const configured = io.env['CAPSULEBIN_SERVER'];
    if (!fromEnv || !configured) return fromEnv || undefined;

    let tokenServer: string | undefined;
    try {
        tokenServer = normalizeBase(configured);
    } catch {
        // Not a URL: the token's server cannot be told, so it matches none.
    }
    const used = normalizeBase(server);
    if (tokenServer === used) return fromEnv;
    io.stderr.write(
        `capsulebin: ${escapeDiagnostic(
            `CAPSULEBIN_TOKEN is only sent to the server in CAPSULEBIN_SERVER ` +
                `(${tokenServer ?? 'not a usable URL'}); the server in use is ${used}, ` +
                `so the token is not sent. Use --token-file for this server`,
        )}\n`,
    );
    return undefined;
}

/**
 * The password for a new capsule, if one was asked for: `--password-file`,
 * else (with `--password`) `CAPSULEBIN_PASSWORD`, else a prompt with
 * confirmation. A password of only whitespace is refused, not dropped: the
 * capsule would otherwise be created without the protection asked for.
 * Anything else is used exactly as given, surrounding spaces included.
 */
export async function newPassword(
    io: Io,
    flags: { password: boolean; passwordFile: string | undefined },
): Promise<string | null> {
    if (flags.passwordFile !== undefined) {
        const line = await readSecretFile(io, flags.passwordFile, 'password');
        if (!line.trim()) {
            throw usage(`password file ${flags.passwordFile} contains only whitespace`);
        }
        return line;
    }
    if (!flags.password) return null;

    const fromEnv = io.env['CAPSULEBIN_PASSWORD'];
    if (fromEnv) {
        if (!fromEnv.trim()) throw usage('CAPSULEBIN_PASSWORD contains only whitespace');
        return fromEnv;
    }

    if (!io.interactive) {
        throw usage('--password needs a terminal; use --password-file or CAPSULEBIN_PASSWORD');
    }
    const first = await io.prompt('Password: ', true);
    if (!first.trim()) throw usage('empty password');
    const second = await io.prompt('Repeat password: ', true);
    if (first !== second) throw usage('passwords do not match');
    return first;
}

/**
 * The password to open a protected capsule: `--password-file` (for any
 * link), else `CAPSULEBIN_PASSWORD`, else a prompt. Without any of them, the
 * command stops before retrieving anything.
 *
 * `CAPSULEBIN_PASSWORD` is only used when the link's server base equals the
 * trusted server (`origin.anchor`, from `--server`/`CAPSULEBIN_SERVER`): the
 * link chooses the KDF parameters, and a reveal token derived from the
 * password lets its server guess the password offline. For any other link
 * the command stops (non-interactive) or prompts instead.
 */
export async function existingPassword(
    io: Io,
    passwordFile: string | undefined,
    origin: { link: string; anchor: string | undefined },
): Promise<string> {
    if (passwordFile !== undefined) return readSecretFile(io, passwordFile, 'password');
    const fromEnv = io.env['CAPSULEBIN_PASSWORD'];
    if (fromEnv) {
        const trusted =
            origin.anchor !== undefined &&
            normalizeBase(origin.anchor) === normalizeBase(origin.link);
        if (trusted) return fromEnv;
        const mismatch =
            `CAPSULEBIN_PASSWORD is only used for links to the server given with --server ` +
            `(or CAPSULEBIN_SERVER); this link's server is ${normalizeBase(origin.link)}` +
            (origin.anchor === undefined
                ? ' and no --server is given'
                : `, --server is ${normalizeBase(origin.anchor)}`);
        if (!io.interactive) {
            throw new CliError(
                `${mismatch}; use --password-file or --server`,
                ExitCode.NeedsInput,
                'password_origin_mismatch',
            );
        }
        io.stderr.write(
            `capsulebin: ${escapeDiagnostic(mismatch)}; the environment password is not used\n`,
        );
        return io.prompt('Password: ', true);
    }
    if (!io.interactive) {
        throw new CliError(
            'this capsule is password-protected; use --password-file or CAPSULEBIN_PASSWORD',
            ExitCode.NeedsInput,
            'password_required',
        );
    }
    return io.prompt('Password: ', true);
}
