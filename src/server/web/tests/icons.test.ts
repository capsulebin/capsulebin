/**
 * src/pages/icons.ts and src/pages/hexagon.ts: decorative inline SVG built as
 * DOM, without markup strings or inline styles, usable from plain DOM and Lit.
 */

import { html, render } from 'lit';
import { describe, expect, it } from 'vitest';

import { type HexagonState, hexagon } from '../src/pages/hexagon';
import { type IconName, icon } from '../src/pages/icons';

const SVG_NS = 'http://www.w3.org/2000/svg';

const ICONS: IconName[] = [
    'copy',
    'check',
    'download',
    'chevron-left',
    'chevron-right',
    'chevron-down',
    'remove',
    'lock',
    'clock',
    'stack',
    'theme',
    'sun',
    'moon',
];

const STATES: HexagonState[] = ['running', 'sealed', 'locked', 'warning', 'empty'];

/** Every element of `svg`, itself included. */
function elements(svg: SVGSVGElement): Element[] {
    return [svg, ...svg.querySelectorAll('*')];
}

function expectDecorativeSvg(svg: SVGSVGElement): void {
    expect(svg.namespaceURI).toBe(SVG_NS);
    expect(svg.tagName).toBe('svg');
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.getAttribute('focusable')).toBe('false');
    expect(svg.textContent).toBe('');
    for (const element of elements(svg)) {
        expect(element.namespaceURI).toBe(SVG_NS);
        expect(element.hasAttribute('style')).toBe(false);
    }
}

describe('icon', () => {
    it.each(ICONS)('draws %s as a decorative SVG', (name) => {
        const svg = icon(name);
        expectDecorativeSvg(svg);
        expect(svg.getAttribute('class')).toBe('cb-icon');
        expect(svg.getAttribute('viewBox')).toBe('0 0 16 16');
        expect(svg.dataset.icon).toBe(name);
        expect(svg.children.length).toBeGreaterThan(0);
    });

    it('returns a new node on every call', () => {
        expect(icon('copy')).not.toBe(icon('copy'));
        expect(icon('copy').outerHTML).toBe(icon('copy').outerHTML);
    });
});

describe('hexagon', () => {
    it.each(STATES)('draws %s with the state in data-hexagon', (state) => {
        const svg = hexagon(state);
        expectDecorativeSvg(svg);
        expect(svg.getAttribute('class')).toBe('cb-hex');
        expect(svg.dataset.hexagon).toBe(state);
        expect(svg.getAttribute('viewBox')).toBe('0 0 18 20');

        // Fill and outline share the path; the outline's length is fixed for the
        // stroke-dashoffset animations in base.css.
        const fill = svg.querySelector('.cb-hex-fill')!;
        const ring = svg.querySelector('.cb-hex-ring')!;
        expect(ring.getAttribute('d')).toBe(fill.getAttribute('d'));
        expect(ring.getAttribute('pathLength')).toBe('60');
    });

    it.each([
        ['running', 0],
        ['sealed', 1],
        ['locked', 1],
        ['warning', 1],
        ['empty', 0],
    ] as const)('draws a glyph inside %s: %i', (state, glyphs) => {
        expect(hexagon(state).querySelectorAll('.cb-hex-glyph')).toHaveLength(glyphs);
    });

    it('draws the lock for sealed and locked, another glyph for warning', () => {
        const glyph = (state: HexagonState) =>
            hexagon(state).querySelector('.cb-hex-glyph')!.getAttribute('d');
        expect(glyph('sealed')).toBe(glyph('locked'));
        expect(glyph('warning')).not.toBe(glyph('locked'));
    });

    it('works in plain DOM and in a Lit template', () => {
        const plain = document.createElement('div');
        plain.append(hexagon('running'));
        expect(plain.querySelector('[data-hexagon="running"]')).not.toBeNull();

        const lit = document.createElement('div');
        const view = (state: HexagonState) =>
            html`<div class="state">${hexagon(state)}${icon('lock')}</div>`;
        render(view('locked'), lit);
        expect(lit.querySelectorAll('[data-hexagon]')).toHaveLength(1);
        expect(lit.querySelector('[data-hexagon="locked"]')).not.toBeNull();
        render(view('empty'), lit);
        expect(lit.querySelectorAll('[data-hexagon]')).toHaveLength(1);
        expect(lit.querySelector('[data-hexagon="empty"]')).not.toBeNull();
        expect(lit.querySelector('[data-icon="lock"]')).not.toBeNull();
    });
});
