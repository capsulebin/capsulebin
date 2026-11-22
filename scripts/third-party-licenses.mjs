#!/usr/bin/env node
//
// Collects the licence notices of the third-party code that CapsuleBin's
// artefacts contain, so each artefact can ship them.
//
//   node scripts/third-party-licenses.mjs server [--target <triple>] [--web <file>]
//   node scripts/third-party-licenses.mjs cli <esbuild metafile> <output file>
//
// - `server`: the Rust crates linked into the `capsulebin` binary for the target
//   (default: the host of `rustc`), followed by the web client's notices (`--web`,
//   the `third-party-licenses.txt` of the web build, which the binary embeds).
//   Written to stdout. Reads `cargo metadata --locked`, follows only normal
//   dependencies from `capsulebin-server` and leaves out build and dev
//   dependencies and proc-macro crates (and what only they pull in): none of it
//   ends up in the binary.
// - `cli`: the npm packages in the CLI bundle, from esbuild's metafile.
// - `npmNotices(moduleIds, title)`: the same for any bundler's list of module
//   paths; the web build (vite.config.ts) emits its file with it.
//
// Every package must ship at least one licence file (LICENSE*, LICENCE*,
// COPYING*, NOTICE*, COPYRIGHT*, UNLICENSE*) - all of them are included, so an
// Apache-2.0 NOTICE travels with its licence - and its declared licence must be
// satisfiable with the `[licenses] allow` list of deny.toml. Anything else fails
// the generation, so a new dependency cannot ship without its notice. Identical
// texts are printed once, with the packages they belong to. The output depends
// only on the inputs (sorted, no timestamps), so the web build stays
// reproducible.

import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const ROOT = join(import.meta.dirname, '..');
const LICENCE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)/i;

/** The licence identifiers deny.toml allows (also used for npm packages). */
export function allowedLicences() {
    const toml = readFileSync(join(ROOT, 'deny.toml'), 'utf8');
    const section = /^\[licenses\]$([\s\S]*?)^\[/m.exec(toml)?.[1] ?? '';
    const list = /^allow\s*=\s*\[([\s\S]*?)\]/m.exec(section)?.[1];
    if (list === undefined) throw new Error('deny.toml: no [licenses] allow list');
    const allowed = new Set([...list.matchAll(/"([^"]+)"/g)].map((match) => match[1]));
    if (allowed.size === 0) throw new Error('deny.toml: the [licenses] allow list is empty');
    return allowed;
}

/**
 * Whether the SPDX expression `expression` can be met with the `allowed`
 * licences: `OR` needs one side, `AND` both, `X WITH Y` is one licence. The
 * old Cargo spelling `A/B` means `A OR B`.
 */
export function satisfiable(expression, allowed) {
    const tokens = expression
        .replaceAll('/', ' OR ')
        .replaceAll('(', ' ( ')
        .replaceAll(')', ' ) ')
        .split(/\s+/)
        .filter(Boolean);
    let at = 0;
    const fail = () => {
        throw new Error(`cannot read the licence expression "${expression}"`);
    };
    const primary = () => {
        const token = tokens[at++];
        if (token === undefined || token === ')' || token === 'AND' || token === 'OR') fail();
        if (token === '(') {
            const inner = or();
            if (tokens[at++] !== ')') fail();
            return inner;
        }
        if (tokens[at] === 'WITH') {
            const exception = tokens[at + 1];
            if (exception === undefined) fail();
            at += 2;
            return allowed.has(`${token} WITH ${exception}`);
        }
        return allowed.has(token);
    };
    const and = () => {
        let result = primary();
        while (tokens[at] === 'AND') {
            at++;
            result = primary() && result;
        }
        return result;
    };
    const or = () => {
        let result = and();
        while (tokens[at] === 'OR') {
            at++;
            result = and() || result;
        }
        return result;
    };
    const result = or();
    if (at !== tokens.length) fail();
    return result;
}

/**
 * The licence files in `dir`, sorted by name: `LICENSE*` and the like, and the
 * files of a REUSE-style `LICENSES/` directory.
 */
function licenceFiles(dir) {
    const files = [];
    for (const name of readdirSync(dir).sort()) {
        if (!LICENCE_FILE.test(name)) continue;
        const path = join(dir, name);
        if (statSync(path).isFile()) {
            files.push({ name, text: normalise(readFileSync(path, 'utf8')) });
        } else if (statSync(path).isDirectory()) {
            for (const inner of readdirSync(path).sort()) {
                if (!statSync(join(path, inner)).isFile()) continue;
                const text = normalise(readFileSync(join(path, inner), 'utf8'));
                files.push({ name: `${name}/${inner}`, text });
            }
        }
    }
    return files.filter((file) => file.text !== '');
}

function normalise(text) {
    return text.replace(/^﻿/, '').replaceAll('\r\n', '\n').trim();
}

/**
 * Checks every package and returns them with their licence files; fails
 * naming every package that breaks a rule.
 * `packages`: `[{ name, version, licence, url, dir }]`.
 */
function withLicenceFiles(packages, allowed) {
    const problems = [];
    const checked = packages.map((pkg) => {
        const label = `${pkg.name} ${pkg.version}`;
        if (!pkg.licence) {
            problems.push(`${label} declares no licence`);
        } else if (!satisfiable(pkg.licence, allowed)) {
            problems.push(`${label}: "${pkg.licence}" is not allowed by deny.toml [licenses]`);
        }
        const files = licenceFiles(pkg.dir);
        if (files.length === 0) problems.push(`${label} ships no licence text in ${pkg.dir}`);
        return { ...pkg, files };
    });
    if (problems.length > 0) throw new Error(problems.join('\n'));
    return checked;
}

/** The notice text for `packages` (already checked), titled `title`. */
function render(title, packages) {
    const sorted = [...packages].sort(
        (a, b) => compare(a.name, b.name) || compare(a.version, b.version),
    );
    const heading = `Third-party software in ${title}`;
    const lines = [heading, '='.repeat(heading.length), ''];
    if (sorted.length === 0) {
        lines.push(`${capitalise(title)} contains no third-party packages.`);
        return `${lines.join('\n')}\n`;
    }
    lines.push(
        `${capitalise(title)} contains the following third-party packages. Their licence`,
        'texts follow the list; a text shared by several packages is printed once.',
        '',
    );
    for (const pkg of sorted) {
        lines.push(`- ${pkg.name} ${pkg.version} (${pkg.licence})${pkg.url ? ` ${pkg.url}` : ''}`);
    }

    // One block per distinct text (whitespace aside), in the order of the first
    // package using it, printed as that package ships it.
    const blocks = new Map();
    for (const pkg of sorted) {
        for (const file of pkg.files) {
            const key = file.text.replace(/\s+/g, ' ');
            const block = blocks.get(key) ?? { text: file.text, users: [], names: new Set() };
            block.users.push(`${pkg.name} ${pkg.version}`);
            block.names.add(file.name);
            blocks.set(key, block);
        }
    }
    for (const { text, ...block } of blocks.values()) {
        lines.push(
            '',
            '-'.repeat(78),
            `${[...block.names].join(', ')} of: ${block.users.join(', ')}`,
        );
        lines.push('-'.repeat(78), '', text);
    }
    return `${lines.join('\n')}\n`;
}

function compare(a, b) {
    return a < b ? -1 : a > b ? 1 : 0;
}

function capitalise(text) {
    return text.charAt(0).toUpperCase() + text.slice(1);
}

/** The npm package directory a module path lies in, or null outside node_modules. */
function npmPackageDir(modulePath) {
    const path = modulePath.replaceAll('\\', '/').split('?')[0];
    const match = /^(.*\/node_modules\/(?:@[^/]+\/)?[^/]+)\//.exec(path);
    return match ? match[1] : null;
}

/**
 * The notice text for the npm packages among `moduleIds` (paths of bundled
 * modules; virtual ids and paths outside node_modules are ignored).
 */
export function npmNotices(moduleIds, title) {
    const allowed = allowedLicences();
    const dirs = new Set();
    for (const id of moduleIds) {
        if (id.startsWith('\0')) continue;
        const dir = npmPackageDir(resolve(id));
        if (dir) dirs.add(dir);
    }
    const found = [...dirs].map((dir) => {
        const manifest = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
        const licence = typeof manifest.license === 'string' ? manifest.license : null;
        const repository =
            typeof manifest.repository === 'string'
                ? manifest.repository
                : manifest.repository?.url;
        return {
            name: manifest.name ?? basename(dir),
            version: manifest.version ?? '',
            licence,
            url: manifest.homepage ?? repository ?? null,
            dir,
        };
    });
    return render(title, withLicenceFiles(found, allowed));
}

/** The crates linked into `capsulebin` for `target`, from `cargo metadata`. */
function rustPackages(target) {
    const metadata = JSON.parse(
        execFileSync(
            'cargo',
            ['metadata', '--locked', '--format-version', '1', '--filter-platform', target],
            { cwd: ROOT, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
        ),
    );
    const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
    const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
    const root = metadata.packages.find((pkg) => pkg.name === 'capsulebin-server');
    if (!root) throw new Error('cargo metadata: no package capsulebin-server');

    const linked = new Set();
    const queue = [root.id];
    while (queue.length > 0) {
        const id = queue.pop();
        if (linked.has(id)) continue;
        linked.add(id);
        for (const dep of nodes.get(id)?.deps ?? []) {
            // `kind: null` is a normal dependency; build and dev ones are not linked.
            if (!dep.dep_kinds.some((kind) => kind.kind === null)) continue;
            const pkg = packages.get(dep.pkg);
            // A proc-macro runs in the compiler; neither it nor its own
            // dependencies end up in the binary.
            if (pkg.targets.some((t) => t.kind.includes('proc-macro'))) continue;
            queue.push(dep.pkg);
        }
    }
    linked.delete(root.id);
    return [...linked].map((id) => {
        const pkg = packages.get(id);
        return {
            name: pkg.name,
            version: pkg.version,
            licence: pkg.license,
            url: pkg.repository ?? pkg.homepage ?? null,
            dir: dirname(pkg.manifest_path),
        };
    });
}

function hostTarget() {
    const info = execFileSync('rustc', ['-vV'], { cwd: ROOT, encoding: 'utf8' });
    const host = /^host: (.+)$/m.exec(info)?.[1];
    if (!host) throw new Error('rustc -vV names no host');
    return host;
}

/** The server binary's notices: its crates, then the embedded web client's. */
export function serverNotices(target, webNoticesFile) {
    const allowed = allowedLicences();
    const crates = withLicenceFiles(rustPackages(target), allowed);
    let text = render(`the CapsuleBin server (${target})`, crates);
    if (webNoticesFile) {
        text += `\n\n${'#'.repeat(78)}\n\nThe server embeds the web client, which it serves to browsers:\n\n`;
        text += readFileSync(webNoticesFile, 'utf8');
    }
    return text;
}

function main(args) {
    const [command, ...rest] = args;
    const option = (name) => {
        const at = rest.indexOf(name);
        if (at === -1) return undefined;
        const value = rest[at + 1];
        if (value === undefined) throw new Error(`${name} needs a value`);
        return value;
    };
    if (command === 'server') {
        process.stdout.write(serverNotices(option('--target') ?? hostTarget(), option('--web')));
    } else if (command === 'cli' && rest.length === 2) {
        const [metafile, output] = rest;
        const meta = JSON.parse(readFileSync(metafile, 'utf8'));
        // Metafile inputs are relative to esbuild's working directory.
        const ids = Object.keys(meta.inputs).map((input) => resolve(input));
        writeFileSync(output, npmNotices(ids, 'the CapsuleBin command line client'));
    } else {
        throw new Error(
            'usage: third-party-licenses.mjs server [--target <triple>] [--web <file>]\n' +
                '       third-party-licenses.mjs cli <metafile> <output>',
        );
    }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    try {
        main(process.argv.slice(2));
    } catch (error) {
        process.stderr.write(`third-party-licenses: ${error.message}\n`);
        process.exit(1);
    }
}
