/**
 * The state hexagon: CapsuleBin's symbol, drawn for a state page.
 *
 * One builder for every page (view, create, manage), returning a DOM node that
 * plain-DOM pages append and Lit templates embed (`${hexagon('running')}`). The
 * state is the `data-hexagon` attribute; base.css draws and animates it with
 * CSS only (stroke-dashoffset on the outline, which has pathLength 60), and
 * switches the animation off for `prefers-reduced-motion`. No inline style.
 */

import { type Shape, drawing } from './icons';

/**
 * - `running`: something is in progress (opening, decrypting, deriving the key,
 *   encrypting, loading, deleting) - a dash runs around the outline.
 * - `sealed`: just created - the outline closes once.
 * - `locked`: a password is needed.
 * - `warning`: an irreversible action awaits confirmation.
 * - `empty`: gone or failed - dashed outline, nothing inside.
 */
export type HexagonState = 'running' | 'sealed' | 'locked' | 'warning' | 'empty';

const OUTLINE = 'M9 1.2 16.4 5.4v9.2L9 18.8 1.6 14.6V5.4L9 1.2Z';

const LOCK: Shape = [
    'path',
    { class: 'cb-hex-glyph', d: 'M6.2 9.4h5.6v4.2H6.2V9.4Zm1.1 0V7.9a1.7 1.7 0 0 1 3.4 0v1.5' },
];

const EXCLAMATION: Shape = ['path', { class: 'cb-hex-glyph', d: 'M9 6.5V11m0 2.3v.2' }];

const GLYPH: Record<HexagonState, Shape | null> = {
    running: null,
    sealed: LOCK,
    locked: LOCK,
    warning: EXCLAMATION,
    empty: null,
};

/** The hexagon for `state`: a decorative `<svg class="cb-hex" data-hexagon="…">`. */
export function hexagon(state: HexagonState): SVGSVGElement {
    const glyph = GLYPH[state];
    const svg = drawing('0 0 18 20', 'cb-hex', [
        ['path', { class: 'cb-hex-fill', d: OUTLINE }],
        ['path', { class: 'cb-hex-ring', d: OUTLINE, pathLength: '60' }],
        ...(glyph ? [glyph] : []),
    ]);
    svg.dataset.hexagon = state;
    return svg;
}
