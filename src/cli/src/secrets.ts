/**
 * Where passwords, creation tokens and links come from.
 *
 * Never a password or token from a command-line argument: argv is visible to
 * every user on the machine (`ps`) and lands in shell history. The sources are
 * a file, an environment variable, or an interactive prompt. A link may still
 * be an argument for compatibility, but can come from stdin or a file instead.
 */

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { CliError, ExitCode, usage } from './exit';
import type { Io } from './io';
import { escapeDiagnostic } from './terminal';
import { normalizeBase } from './transport';

/** The first line of `content`, without its line ending. */
function firstLine(content: string): string {
    return content.split(/\r?\n/, 1)[0] ?? '';
}

/** First line of a file, without the trailing newline. */
async function readSecretFile(io: Io, path: string, what: string): Promise<string> {
    let content: string;
    try {
        content = await readFile(resolve(io.cwd, path), 'utf8');
    } catch {
        throw usage(`cannot read ${what} file ${path}`);
    }
    const line = firstLine(content);
    if (!line) throw usage(`${what} file ${path} is empty`);
    return line;
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
    const line = firstLine(new TextDecoder().decode(await io.readStdin()));
    if (!line) throw usage('no link on stdin');
    return line;
}

/** The creation token: `--token-file`, else `CAPSULEBIN_TOKEN`, else none. */
export async function creationToken(
    io: Io,
    tokenFile: string | undefined,
): Promise<string | undefined> {
    if (tokenFile) return readSecretFile(io, tokenFile, 'token');
    return io.env['CAPSULEBIN_TOKEN'] || undefined;
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
    if (flags.passwordFile) {
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
    if (passwordFile) return readSecretFile(io, passwordFile, 'password');
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
