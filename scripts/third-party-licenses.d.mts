// Types for the parts of third-party-licenses.mjs that the web build
// (vite.config.ts) and its tests import.

/** The licence identifiers deny.toml's `[licenses] allow` list names. */
export function allowedLicences(): Set<string>;

/** Whether the SPDX expression can be met with the `allowed` licences. */
export function satisfiable(expression: string, allowed: ReadonlySet<string>): boolean;

/**
 * The notice text for the npm packages among `moduleIds` (paths of bundled
 * modules); throws when a package ships no licence text or declares a licence
 * deny.toml does not allow.
 */
export function npmNotices(moduleIds: Iterable<string>, title: string): string;
