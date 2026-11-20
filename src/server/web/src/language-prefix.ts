/**
 * The one rule for the path prefix of a language: pages in the default language
 * have none (`/report`), pages in every other language sit under `/<lang>`
 * (`/de/report`). The server applies the same rule (`language_prefix()` in
 * `config/model.rs`). Kept free of imports and of the DOM so that the page
 * modules, the Node.js scripts and the tests all share it.
 */
export function languagePrefix(defaultLanguage: string, language: string): string {
    return language === defaultLanguage ? '' : `/${language}`;
}
