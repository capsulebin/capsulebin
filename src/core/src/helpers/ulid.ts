/**
 * ULID generator: 26-character Crockford-base32 identifiers with a millisecond
 * timestamp prefix and 80 bits of randomness.
 *
 * Specification: https://github.com/ulid/spec
 */

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const CROCKFORD_LOOKUP: Record<string, number> = {};
for (let i = 0; i < CROCKFORD.length; i++) {
    CROCKFORD_LOOKUP[CROCKFORD[i]!] = i;
}

/**
 * Generates a ULID.
 *
 *  - 48-bit timestamp (high 10 characters)
 *  - 80-bit random (low 16 characters)
 *
 * The random component uses `crypto.getRandomValues`.
 */
export function generateUlid(): string {
    const now = Date.now();
    const random = crypto.getRandomValues(new Uint8Array(10));

    // Timestamp: 48 bits → 10 Crockford characters.
    const timeChars = new Array<string>(10);
    let t = now;
    for (let i = 9; i >= 0; i--) {
        timeChars[i] = CROCKFORD[t & 0x1f]!;
        t >>>= 5;
    }

    // Random: 80 bits → 16 Crockford characters (each byte contributes 1.6 chars,
    // so encode 5 bits at a time across the 10-byte buffer = 16 × 5 = 80 bits).
    const randChars = new Array<string>(16);
    let bitPos = 0;
    for (let i = 0; i < 16; i++) {
        const byteIdx = bitPos >>> 3;
        const bitOff = bitPos & 7;
        const value =
            bitOff <= 3
                ? (random[byteIdx]! >>> (3 - bitOff)) & 0x1f
                : ((random[byteIdx]! << (bitOff - 3)) | (random[byteIdx + 1]! >>> (11 - bitOff))) &
                  0x1f;
        randChars[i] = CROCKFORD[value]!;
        bitPos += 5;
    }

    return timeChars.join('') + randChars.join('');
}
