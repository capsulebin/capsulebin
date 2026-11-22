/**
 * Every catalogue key is still used by a page: keys whose text the refreshed
 * pages replaced are removed, not left behind. A key counts as used when its
 * name appears in a source file, a shell template or a head script. The key
 * part note names no colour, because operators can change the accent.
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import de from '../src/i18n/de.json';
import en from '../src/i18n/en.json';

const ROOT = join(import.meta.dirname, '..');

function files(directory: string): string[] {
    return readdirSync(directory).flatMap((name) => {
        const path = join(directory, name);
        return statSync(path).isDirectory() ? files(path) : [path];
    });
}

const sources = [
    ...files(join(ROOT, 'src')).filter((path) => !path.endsWith('.json')),
    ...files(join(ROOT, 'scripts')),
    ...files(join(ROOT, 'public')).filter((path) => path.endsWith('.js')),
    ...['index.html', 'p.html', 'manage.html', 'report.html', 'legal.html'].map((name) =>
        join(ROOT, name),
    ),
]
    .map((path) => readFileSync(path, 'utf8'))
    .join('\n');

describe('catalogue keys', () => {
    it('are all referenced by a page, a shell or a script', () => {
        const keys = [...new Set(Object.keys(en).map((key) => key.split('#')[0]!))];
        expect(keys.filter((key) => !sources.includes(key))).toEqual([]);
    });

    it('no longer hold the replaced "more" texts', () => {
        for (const catalogue of [en, de] as Record<string, string>[]) {
            for (const key of [
                'items.more',
                'create.more',
                'view.copied',
                'view.copiedToClipboard',
            ])
                expect(Object.keys(catalogue).filter((name) => name.split('#')[0] === key)).toEqual(
                    [],
                );
        }
    });

    it('describe the key part without naming a colour', () => {
        const colours = /\b(blue|green|red|orange|purple|teal|blau|grün|rot|orange|lila|türkis)\b/i;
        for (const catalogue of [en, de] as Record<string, string>[]) {
            expect(catalogue['create.keyNote']).toBeTruthy();
            expect(catalogue['create.keyNote']).not.toMatch(colours);
        }
        expect(en['create.keyNote' as keyof typeof en]).toMatch(/highlighted part/i);
    });
});
