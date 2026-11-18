/**
 * Where passwords and creation tokens come from.
 *
 * Never from a command-line argument: argv is visible to every user on the
 * machine (`ps`) and lands in shell history. The sources are a file, an
 * environment variable, or an interactive prompt.
 */

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { CliError, ExitCode, usage } from './exit';
import type { Io } from './io';

/** First line of a file, without the trailing newline. */
async function readSecretFile(io: Io, path: string, what: string): Promise<string> {
    let content: string;
    try {
        content = await readFile(resolve(io.cwd, path), 'utf8');
    } catch {
        throw usage(`cannot read ${what} file ${path}`);
    }
    const line = content.split(/\r?\n/, 1)[0] ?? '';
    if (!line) throw usage(`${what} file ${path} is empty`);
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
 * confirmation.
 */
export async function newPassword(
    io: Io,
    flags: { password: boolean; passwordFile: string | undefined },
): Promise<string | null> {
    if (flags.passwordFile) return readSecretFile(io, flags.passwordFile, 'password');
    if (!flags.password) return null;

    const fromEnv = io.env['CAPSULEBIN_PASSWORD'];
    if (fromEnv) return fromEnv;

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
 * The password to open a protected capsule: `--password-file`, else
 * `CAPSULEBIN_PASSWORD`, else a prompt. Without any of them, the command
 * stops before retrieving anything.
 */
export async function existingPassword(io: Io, passwordFile: string | undefined): Promise<string> {
    if (passwordFile) return readSecretFile(io, passwordFile, 'password');
    const fromEnv = io.env['CAPSULEBIN_PASSWORD'];
    if (fromEnv) return fromEnv;
    if (!io.interactive) {
        throw new CliError(
            'this capsule is password-protected; use --password-file or CAPSULEBIN_PASSWORD',
            ExitCode.NeedsInput,
            'password_required',
        );
    }
    return io.prompt('Password: ', true);
}
