/**
 * The production build of the web client: one Vite build per interface language
 * (`npm run build`).
 *
 * Each build takes the shells of its language (scripts/shells.ts) and the
 * catalogue of its language as `virtual:catalogue` (scripts/catalogue.ts), and
 * writes `dist/<lang>/*.html` plus its chunks into the shared `dist/assets/`.
 * Chunk names carry a content hash, so a chunk that does not depend on the
 * language comes out the same in every build and exists once. The build then
 * checks that (scripts/check-dist.ts) and fails on a finding.
 */

import { rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { build } from 'vite';

import { catalogueModule, templateOnlyKeys } from './catalogue.ts';
import { checkDist } from './check-dist.ts';
import {
    SHELLS,
    catalogueLanguages,
    generateShells,
    generatedPath,
    readCatalogue,
} from './shells.ts';

const WEB_ROOT = resolve(import.meta.dirname, '..');
const OUT_DIR = join(WEB_ROOT, 'dist');

rmSync(OUT_DIR, { recursive: true, force: true });
generateShells();

const languages = catalogueLanguages();
for (const lang of languages) {
    await build({
        configFile: join(WEB_ROOT, 'vite.config.ts'),
        plugins: [catalogueModule(lang)],
        build: {
            // Emptied once above; every language adds its files.
            emptyOutDir: false,
            rolldownOptions: {
                input: SHELLS.map((shell) => join(WEB_ROOT, generatedPath(lang, shell))),
            },
        },
    });
}

const problems = checkDist(OUT_DIR, {
    catalogues: Object.fromEntries(languages.map((lang) => [lang, readCatalogue(lang)])),
    templateOnlyKeys: [...new Set(languages.flatMap(templateOnlyKeys))],
});
if (problems.length > 0) {
    console.error(`the per-language build output is wrong:\n  ${problems.join('\n  ')}`);
    process.exit(1);
}
console.log(`ok  ${languages.join(', ')}: every page loads its own language only`);
