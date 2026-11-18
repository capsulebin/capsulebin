// @vitest-environment node
//
// Reads the two config files from disk and needs no DOM.

/**
 * Keeps the TypeScript library level and the browser baseline in step.
 *
 * These two settings look independent and are not. `tsconfig.json` decides which
 * APIs TypeScript believes exist at runtime; `vite.config.ts` decides which
 * browsers the output has to run on. esbuild rewrites *syntax* to match the
 * browser target but does **not** polyfill *APIs* — so raising `lib` alone does
 * not make a newer method available, it only removes the compile error that was
 * protecting users on older browsers from a runtime `TypeError`.
 *
 * That makes `lib` a promise about the oldest supported browser, not a statement
 * about how modern the codebase is. This test exists so that raising one setting
 * forces a decision about the other instead of silently narrowing the audience.
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * The agreed baseline. Changing these two values together is a deliberate
 * browser-support decision; changing one alone is a bug.
 *
 * Safari is the binding constraint: Safari 16 shipped in September 2022, which is
 * what makes ES2022 the right library level. ES2023 introduced `Array.prototype`
 * copy methods and ES2024 `Object.groupBy` and `Promise.withResolvers`, none of
 * which Safari 16 has.
 */
const BASELINE = {
    tsconfigTarget: 'ES2022',
    tsconfigLib: 'ES2022',
    browsers: ['chrome107', 'edge107', 'firefox104', 'safari16'],
} as const;

function read(name: string): string {
    return readFileSync(fileURLToPath(new URL(`../${name}`, import.meta.url)), 'utf8');
}

describe('browser baseline', () => {
    it('tsconfig targets the agreed language level', () => {
        // Parsed with a regex rather than JSON.parse: tsconfig.json permits comments,
        // and a future comment should not break this test.
        const tsconfig = read('tsconfig.json');

        const target = /"target"\s*:\s*"([^"]+)"/.exec(tsconfig)?.[1];
        expect(target, 'tsconfig.json has no "target"').toBe(BASELINE.tsconfigTarget);
    });

    it('tsconfig claims no library newer than the browsers provide', () => {
        const tsconfig = read('tsconfig.json');

        const lib = /"lib"\s*:\s*\[([^\]]+)\]/.exec(tsconfig)?.[1];
        expect(lib, 'tsconfig.json has no "lib"').toBeDefined();

        const entries = (lib ?? '')
            .split(',')
            .map((entry) => entry.trim().replace(/"/g, ''))
            .filter(Boolean);

        expect(entries).toContain(BASELINE.tsconfigLib);

        // The failure this guards against: an ES20xx entry above the baseline. DOM
        // entries are unrelated and stay allowed.
        const ecmaScript = entries.filter((entry) => /^ES\d{4}$/i.test(entry));
        for (const entry of ecmaScript) {
            const year = Number.parseInt(entry.slice(2), 10);
            const baselineYear = Number.parseInt(BASELINE.tsconfigLib.slice(2), 10);
            expect(
                year,
                `lib "${entry}" is newer than the browser baseline (${BASELINE.browsers.join(', ')}). ` +
                    'esbuild does not polyfill APIs, so this would ship code that throws for some users. ' +
                    'Raise vite.config.ts build.target in the same change, or lower this entry.',
            ).toBeLessThanOrEqual(baselineYear);
        }
    });

    it('vite pins the browser target instead of inheriting a default', () => {
        const config = read('vite.config.ts');

        // An inherited default would move the baseline whenever Vite is updated.
        expect(config, 'vite.config.ts does not set build.target').toMatch(/target:\s*\[/);

        for (const browser of BASELINE.browsers) {
            expect(config, `build.target is missing ${browser}`).toContain(`'${browser}'`);
        }
    });

    /**
     * `esnext` would defeat the purpose entirely: it claims every proposal-stage API
     * exists, which no shipping browser guarantees.
     */
    it('neither config uses esnext as a runtime promise', () => {
        const tsconfig = read('tsconfig.json');

        const target = /"target"\s*:\s*"([^"]+)"/.exec(tsconfig)?.[1] ?? '';
        const lib = /"lib"\s*:\s*\[([^\]]+)\]/.exec(tsconfig)?.[1] ?? '';

        expect(target.toLowerCase()).not.toBe('esnext');
        expect(lib.toLowerCase()).not.toContain('esnext');
    });
});
