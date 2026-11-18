/**
 * Exit codes and the mapping from failures to them.
 *
 * The codes are part of the CLI's interface: scripts branch on them, so they
 * only ever gain new values. "Not available" deliberately covers unknown,
 * expired, deleted, wrong link and wrong password alike — the server answers
 * all of them identically (ADR-001), and so does the CLI.
 */

import { ApiError } from '@capsulebin/core';

export const ExitCode = {
    Ok: 0,
    /** Unexpected failure (a bug, a local I/O error). */
    Failure: 1,
    /** Bad arguments or input; nothing was sent. */
    Usage: 2,
    /** The capsule is not available: unknown, expired, deleted, wrong link or password. */
    NotAvailable: 3,
    /**
     * A burn-after-retrieval capsule that another retrieval got first (the
     * server's 410 for a lost race). A capsule consumed earlier is reported
     * as {@link NotAvailable}: the server does not reveal that it existed.
     */
    AlreadyRetrieved: 4,
    /** Retrieved, but could not be decrypted or is not a valid capsule. */
    Corrupt: 5,
    /** The server refused the request (unauthorized, too large, rate limited, …). */
    Rejected: 6,
    /** No usable response: network failure or a server error. */
    Unreachable: 7,
    /** Stopped before retrieval: a password or burn confirmation was needed but not given. */
    NeedsInput: 8,
} as const;

export type ExitCode = (typeof ExitCode)[keyof typeof ExitCode];

/** A failure with a message for stderr and the exit code to end with. */
export class CliError extends Error {
    constructor(
        message: string,
        public readonly exitCode: ExitCode,
        /** Machine-readable reason for `--json` output. */
        public readonly reason: string,
    ) {
        super(message);
        this.name = 'CliError';
    }
}

export function usage(message: string): CliError {
    return new CliError(message, ExitCode.Usage, 'usage');
}

/** Maps an API failure to a CLI error. Never includes request bodies or secrets. */
export function fromApiError(err: ApiError): CliError {
    if (err.status === 0) {
        return new CliError('could not reach the server', ExitCode.Unreachable, 'network_error');
    }
    if (err.status >= 500) {
        return new CliError(
            `server error (${err.status}${err.code ? `, ${err.code}` : ''})`,
            ExitCode.Unreachable,
            err.code ?? 'server_error',
        );
    }
    if (err.status === 404) {
        return new CliError(
            'the capsule is not available — it may have expired or been deleted, or the link or password is wrong',
            ExitCode.NotAvailable,
            'not_available',
        );
    }
    if (err.status === 410) {
        return alreadyRetrieved();
    }
    const hint = REJECTION_HINTS[err.code ?? ''];
    const retry =
        err.status === 429 && err.retryAfter !== null ? ` (retry after ${err.retryAfter}s)` : '';
    return new CliError(
        `${hint ?? err.message}${retry}`,
        ExitCode.Rejected,
        err.code ?? `http_${err.status}`,
    );
}

export function alreadyRetrieved(): CliError {
    return new CliError(
        'this capsule could only be retrieved once and has already been retrieved',
        ExitCode.AlreadyRetrieved,
        'already_retrieved',
    );
}

const REJECTION_HINTS: Record<string, string> = {
    unauthorized:
        'the server requires a creation token — pass --token-file or set CAPSULEBIN_TOKEN',
    creation_disabled: 'this server does not accept new capsules',
    payload_too_large: 'the capsule is larger than this server accepts',
    expiry_too_long: 'the requested expiry is longer than this server allows',
    permanent_not_allowed: 'this server does not allow capsules that never expire',
    rate_limited: 'too many requests — try again later',
};

/** Turns anything thrown into a CliError. */
export function toCliError(err: unknown): CliError {
    if (err instanceof CliError) return err;
    if (err instanceof ApiError) return fromApiError(err);
    const message = err instanceof Error ? err.message : String(err);
    return new CliError(message, ExitCode.Failure, 'error');
}
