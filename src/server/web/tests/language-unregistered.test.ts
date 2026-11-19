/**
 * src/language.ts before any catalogue module has run: a text lookup fails with
 * a clear error instead of showing keys. A file of its own, so that no other
 * test has registered a catalogue with the module.
 */

import { describe, expect, it } from 'vitest';

import { t, tn } from '../src/language';

describe('texts without a registered catalogue', () => {
    it('fail and name the missing catalogue module', () => {
        expect(() => t('expiry.none')).toThrow(/No catalogue registered/);
        expect(() => tn('expiry.inMinutes', 1)).toThrow(/No catalogue registered/);
    });
});
