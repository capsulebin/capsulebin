/**
 * The expiry choices of the create page, from the instance's expiry policy that
 * the server put into `<meta name="capsulebin-expiry">` (`pastes.default_expiry`,
 * `pastes.maximum_expiry` in whole seconds, `pastes.allow_permanent`).
 *
 * The page offers the familiar steps up to the maximum, the maximum and the
 * default themselves if they are no step, and "No expiry" only where the
 * instance keeps capsules without one. Option values are whole seconds as
 * decimal strings, `''` for no expiry. The server still decides.
 */

import { t, tn, type PluralKey, type TextKey } from './language';

/** What the instance accepts as an expiry. */
export interface ExpiryPolicy {
    /** Preselected expiry in seconds. */
    defaultSeconds: number;
    /** Longest expiry in seconds the server accepts. */
    maximumSeconds: number;
    /** Whether a capsule without expiry is accepted. */
    permanent: boolean;
}

/**
 * The configuration's built-in defaults (7 days, at most 90 days, no capsule
 * without expiry): what an unconfigured server accepts, used for a missing or
 * unusable meta so the page never offers a choice such a server refuses.
 */
export const BUILT_IN_EXPIRY: Readonly<ExpiryPolicy> = {
    defaultSeconds: 604_800,
    maximumSeconds: 7_776_000,
    permanent: false,
};

/** The fixed steps with their own labels. */
const STEPS: ReadonlyMap<number, TextKey> = new Map([
    [3_600, 'create.expiry.hour'],
    [86_400, 'create.expiry.day'],
    [604_800, 'create.expiry.week'],
    [2_592_000, 'create.expiry.30days'],
    [7_776_000, 'create.expiry.90days'],
]);

/** Units for any other value, largest first. */
const UNITS: readonly (readonly [number, PluralKey])[] = [
    [86_400, 'create.expiry.days'],
    [3_600, 'create.expiry.hours'],
    [60, 'create.expiry.minutes'],
    [1, 'create.expiry.seconds'],
];

/** `value` as a positive safe integer if it is written in decimal digits only, else `null`. */
function positiveInteger(value: string | null): number | null {
    if (value === null || !/^[0-9]+$/.test(value)) return null;
    const n = Number(value);
    return Number.isSafeInteger(n) && n > 0 ? n : null;
}

/**
 * The policy from the create page's meta, or {@link BUILT_IN_EXPIRY} if the meta
 * is missing, a value is no positive whole number of seconds, the default exceeds
 * the maximum or `data-permanent` is neither `true` nor `false`.
 */
export function expiryPolicy(): ExpiryPolicy {
    const meta = document.querySelector<HTMLMetaElement>('meta[name="capsulebin-expiry"]');
    if (meta === null) return { ...BUILT_IN_EXPIRY };
    const defaultSeconds = positiveInteger(meta.getAttribute('data-default'));
    const maximumSeconds = positiveInteger(meta.getAttribute('data-maximum'));
    const permanent = meta.getAttribute('data-permanent');
    if (
        defaultSeconds === null ||
        maximumSeconds === null ||
        defaultSeconds > maximumSeconds ||
        (permanent !== 'true' && permanent !== 'false')
    ) {
        return { ...BUILT_IN_EXPIRY };
    }
    return { defaultSeconds, maximumSeconds, permanent: permanent === 'true' };
}

/**
 * The option values for `policy`, in order: the steps up to the maximum plus the
 * maximum and the default, ascending, then `''` if capsules without expiry are allowed.
 */
export function expiryValues(policy: ExpiryPolicy): string[] {
    const seconds = new Set([...STEPS.keys()].filter((step) => step <= policy.maximumSeconds));
    seconds.add(policy.maximumSeconds);
    seconds.add(policy.defaultSeconds);
    const values = [...seconds].sort((a, b) => a - b).map(String);
    return policy.permanent ? [...values, ''] : values;
}

/**
 * The label of an option value: a step's own label, "No expiry" for `''`, any
 * other value in the largest unit that divides it exactly ("14 days", "90 seconds").
 */
export function expiryLabel(value: string): string {
    if (value === '') return t('expiry.none');
    const seconds = Number(value);
    const step = STEPS.get(seconds);
    if (step !== undefined) return t(step);
    // Seconds divide every whole number, so a unit is always found.
    const [size, key] = UNITS.find(([unit]) => seconds % unit === 0)!;
    return tn(key, seconds / size);
}

/**
 * Replaces the options of `select` with those of `policy`, the default selected
 * and the only one marked `defaultSelected` (a draft is what differs from it).
 */
export function fillExpirySelect(select: HTMLSelectElement, policy: ExpiryPolicy): void {
    const defaultValue = String(policy.defaultSeconds);
    select.replaceChildren(
        ...expiryValues(policy).map((value) => {
            const option = document.createElement('option');
            option.value = value;
            option.textContent = expiryLabel(value);
            option.defaultSelected = value === defaultValue;
            return option;
        }),
    );
    select.value = defaultValue;
}
