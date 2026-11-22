// @vitest-environment node
//
// The licence notice generator (scripts/third-party-licenses.mjs) against small
// package trees in a temporary directory, the licence rule against deny.toml,
// and (if built) the notices of the real web bundle in dist.

/**
 * Every artefact ships the licence notices of the third-party code in it. The
 * web build emits them as `dist/third-party-licenses.txt`, which the server
 * embeds and serves; a package without a licence text, or with a licence that
 * deny.toml does not allow, fails the build.
 */

import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';

import {
    allowedLicences,
    npmNotices,
    satisfiable,
} from '../../../../scripts/third-party-licenses.mjs';

const NOTICES = fileURLToPath(new URL('../dist/third-party-licenses.txt', import.meta.url));

describe('licence expressions', () => {
    const allowed = new Set(['MIT', 'Apache-2.0', 'Apache-2.0 WITH LLVM-exception']);

    it.each([
        ['MIT', true],
        ['GPL-3.0-only', false],
        ['MIT OR GPL-3.0-only', true],
        ['GPL-3.0-only OR MIT', true],
        ['MIT AND GPL-3.0-only', false],
        ['MIT AND Apache-2.0', true],
        ['(MPL-2.0 OR Apache-2.0)', true],
        ['Apache-2.0/MIT', true],
        ['Unlicense/MIT', true],
        ['Apache-2.0 WITH LLVM-exception', true],
        ['MIT WITH Some-exception', false],
        ['(GPL-2.0-only OR MIT) AND Apache-2.0', true],
        ['GPL-2.0-only OR (MIT AND BSL-1.0)', false],
    ])('%s → %s', (expression, expected) => {
        expect(satisfiable(expression, allowed)).toBe(expected);
    });

    it.each(['', 'MIT OR', '(MIT', 'MIT)', 'AND MIT', 'MIT WITH'])('refuses "%s"', (expression) => {
        expect(() => satisfiable(expression, allowed)).toThrow(/licence expression/);
    });

    it('takes the allowed licences from deny.toml', () => {
        const fromDeny = allowedLicences();
        expect(fromDeny.has('MIT')).toBe(true);
        expect(fromDeny.has('Apache-2.0')).toBe(true);
        expect(fromDeny.has('GPL-3.0-only')).toBe(false);
    });
});

describe('npm notices', () => {
    let root: string | undefined;

    afterEach(() => {
        if (root) rmSync(root, { recursive: true, force: true });
        root = undefined;
    });

    /** A package under `<root>/node_modules/<name>` with the given files; returns a module path in it. */
    function pkg(name: string, manifest: object, files: Record<string, string>): string {
        root ??= mkdtempSync(join(tmpdir(), 'capsulebin-licences-'));
        const dir = join(root, 'node_modules', name);
        mkdirSync(join(dir, 'lib'), { recursive: true });
        writeFileSync(join(dir, 'package.json'), JSON.stringify({ name, ...manifest }));
        for (const [file, text] of Object.entries(files)) writeFileSync(join(dir, file), text);
        writeFileSync(join(dir, 'lib', 'index.js'), '');
        return join(dir, 'lib', 'index.js');
    }

    it('lists every package once and prints every licence file, shared texts once', () => {
        const mit = 'MIT License\n\nCopyright (c) Someone\n';
        const modules = [
            pkg('zeta', { version: '2.0.0', license: 'MIT' }, { LICENSE: mit }),
            pkg(
                '@scope/alpha',
                { version: '1.0.0', license: '(MIT OR Apache-2.0)' },
                {
                    'LICENSE-MIT': mit,
                    'LICENSE-APACHE': 'Apache License\nVersion 2.0',
                    NOTICE: 'Alpha notice',
                },
            ),
            '\0virtual-module',
            '/somewhere/outside/the/tree.js',
        ];
        modules.push(modules[0]!);
        const text = npmNotices(modules, 'the test bundle');

        expect(text).toMatch(/^Third-party software in the test bundle\n=+\n/);
        expect(text).toContain('- @scope/alpha 1.0.0 ((MIT OR Apache-2.0))');
        expect(text).toContain('- zeta 2.0.0 (MIT)');
        expect(text.indexOf('@scope/alpha 1.0.0 (')).toBeLessThan(text.indexOf('zeta 2.0.0 ('));
        expect(text).toContain('Apache License\nVersion 2.0');
        expect(text).toContain('Alpha notice');
        // The MIT text both ship is printed once, naming both.
        expect(text.split('Copyright (c) Someone')).toHaveLength(2);
        expect(text).toContain('LICENSE-MIT, LICENSE of: @scope/alpha 1.0.0, zeta 2.0.0');
    });

    it('says so when the bundle has no third-party package', () => {
        expect(npmNotices(['/src/own.ts'], 'the test bundle')).toContain(
            'The test bundle contains no third-party packages.',
        );
    });

    it('fails for a package without a licence text, naming it', () => {
        const module = pkg('bare', { version: '1.0.0', license: 'MIT' }, { 'README.md': 'hi' });
        expect(() => npmNotices([module], 'x')).toThrow(/bare 1\.0\.0 ships no licence text/);
    });

    it('fails for a licence deny.toml does not allow, and for none', () => {
        const gpl = pkg(
            'copyleft',
            { version: '1.0.0', license: 'GPL-3.0-only' },
            { LICENSE: 'GPL' },
        );
        const none = pkg('unlabelled', { version: '1.0.0' }, { LICENSE: 'text' });
        expect(() => npmNotices([gpl, none], 'x')).toThrow(
            /copyleft 1\.0\.0: "GPL-3\.0-only" is not allowed[\s\S]*unlabelled 1\.0\.0 declares no licence/,
        );
    });
});

describe.runIf(existsSync(NOTICES))('the built web bundle', () => {
    it('carries the notices of the packages it bundles', () => {
        const text = readFileSync(NOTICES, 'utf8');
        for (const name of ['dompurify', 'highlight.js', 'lit', 'lit-html', 'marked']) {
            expect(text).toMatch(new RegExp(`^- ${name.replace('.', '\\.')} \\d`, 'm'));
        }
        // Our own workspace package is not third-party.
        expect(text).not.toContain('@capsulebin/');
    });
});
