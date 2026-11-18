#!/usr/bin/env node
//
// Checks that the runtimes CapsuleBin builds on are still supported, and that
// every place naming them agrees.
//
//   node scripts/support-windows.mjs
//
// - Node.js: the major in .node-version must match the Dockerfile's build image,
//   @types/node, the CLI's esbuild target and the `engines` fields. Its end of
//   life comes from the Node.js release schedule.
// - Debian: the release behind the Dockerfile's base images (bookworm/debian12)
//   must be the same for all of them. Its end of security support (including
//   LTS) comes from endoflife.date.
//
// A mismatch fails. An end of support closer than `warnDays` is a warning,
// closer than `failDays` a failure, so there is time to move before it arrives.
// If a schedule cannot be fetched, that is a warning: an outage elsewhere must
// not block every pull request.

import { readFileSync, appendFileSync } from 'node:fs';
import { join } from 'node:path';

const root = join(import.meta.dirname, '..');
const read = (path) => readFileSync(join(root, path), 'utf8');
const json = (path) => JSON.parse(read(path));

const DAY = 86_400_000;
const errors = [];
const warnings = [];
const rows = [];

// --- what the repository uses ---------------------------------------------------

const nodeMajor = read('.node-version').trim().split('.')[0];
if (!/^\d+$/.test(nodeMajor)) {
    errors.push(`.node-version does not name a Node.js major version: "${nodeMajor}"`);
}

const dockerfile = read('Dockerfile');
const expectEqual = (what, found) => {
    if (found !== nodeMajor) {
        errors.push(
            `${what} uses Node.js ${found ?? '(none found)'}, .node-version says ${nodeMajor}`,
        );
    }
};

expectEqual('The Dockerfile build image', dockerfile.match(/FROM \S*\/node:(\d+)-/)?.[1]);
for (const manifest of [
    'src/core/package.json',
    'src/cli/package.json',
    'src/server/web/package.json',
]) {
    const range = json(manifest).devDependencies?.['@types/node'];
    expectEqual(`@types/node in ${manifest}`, range?.match(/^\^?(\d+)\./)?.[1]);
}
const cli = json('src/cli/package.json');
expectEqual('The CLI build target', cli.scripts?.build?.match(/--target=node(\d+)/)?.[1]);
expectEqual('engines.node in src/cli/package.json', cli.engines?.node?.match(/^>=(\d+)$/)?.[1]);
expectEqual(
    'engines.node in package.json',
    json('package.json').engines?.node?.match(/^>=(\d+)$/)?.[1],
);

const DEBIAN = { bookworm: '12', trixie: '13', forky: '14' };
const debianReleases = new Set([
    ...[...dockerfile.matchAll(/FROM \S+:\S*-(bookworm|trixie|forky)\b/g)].map((m) => DEBIAN[m[1]]),
    ...[...dockerfile.matchAll(/FROM \S*distroless\/\S+-debian(\d+)\b/g)].map((m) => m[1]),
]);
if (debianReleases.size !== 1) {
    errors.push(
        `The Dockerfile's base images name different Debian releases: ${[...debianReleases].join(', ') || 'none'}`,
    );
}
const debian = [...debianReleases][0];

// --- support windows ------------------------------------------------------------

async function fetchJson(url) {
    const response = await fetch(url, { signal: AbortSignal.timeout(15_000) });
    if (!response.ok) {
        throw new Error(`${url} answered ${response.status}`);
    }
    return response.json();
}

function judge(name, end, source, { warnDays, failDays }) {
    const days = Math.floor((Date.parse(end) - Date.now()) / DAY);
    let status = 'ok';
    if (days <= failDays) {
        status = 'failure';
        errors.push(
            `${name} reaches the end of support on ${end} (${days} days); move to a newer release`,
        );
    } else if (days <= warnDays) {
        status = 'warning';
        warnings.push(
            `${name} reaches the end of support on ${end} (${days} days); plan the move to a newer release`,
        );
    }
    rows.push(`| ${name} | ${end} | ${days} | ${status} | ${source} |`);
}

if (errors.length === 0) {
    try {
        const schedule = await fetchJson(
            'https://raw.githubusercontent.com/nodejs/Release/main/schedule.json',
        );
        const entry = schedule[`v${nodeMajor}`];
        if (!entry?.end) {
            errors.push(`The Node.js release schedule has no end date for v${nodeMajor}`);
        } else {
            judge(`Node.js ${nodeMajor}`, entry.end, 'nodejs/Release schedule', {
                warnDays: 180,
                failDays: 90,
            });
        }
    } catch (error) {
        warnings.push(`Could not check the Node.js support window: ${error.message}`);
    }

    try {
        const release = await fetchJson(
            `https://endoflife.date/api/v1/products/debian/releases/${debian}`,
        );
        const end = release.result?.eolFrom;
        if (!end) {
            errors.push(`endoflife.date has no end of support for Debian ${debian}`);
        } else {
            // Moving the base images to a newer Debian raises the glibc the release
            // binaries need, which is a decision of its own - hence the longer lead.
            judge(`Debian ${debian} (base images)`, end, 'endoflife.date, incl. LTS', {
                warnDays: 365,
                failDays: 180,
            });
        }
    } catch (error) {
        warnings.push(`Could not check the Debian support window: ${error.message}`);
    }
}

// --- report ---------------------------------------------------------------------

for (const message of warnings) console.log(`::warning::${message}`);
for (const message of errors) console.log(`::error::${message}`);

const summary = [
    '### Support windows',
    '',
    '| Runtime | End of support | Days left | Status | Source |',
    '| --- | --- | --- | --- | --- |',
    ...rows,
    '',
].join('\n');
console.log(summary);
if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary + '\n');
}

process.exitCode = errors.length > 0 ? 1 : 0;
