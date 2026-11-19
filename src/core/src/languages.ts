/**
 * Interface languages an instance can offer, as the path prefix (`/en/…`) and
 * BCP 47 tag. The same list exists in the server and as the web client's
 * catalogue files; tests keep the three in step, so adding a language means a
 * catalogue file and the code here and in the server.
 */

export const KNOWN_LANGUAGES = ['en', 'de'] as const;

export type Language = (typeof KNOWN_LANGUAGES)[number];

/** True if `code` is one of {@link KNOWN_LANGUAGES} (exact, lowercase). */
export function isKnownLanguage(code: string): code is Language {
    return (KNOWN_LANGUAGES as readonly string[]).includes(code);
}
