/**
 * Inline SVG icons, built as DOM nodes.
 *
 * The CSP allows no external resource and Trusted Types forbid HTML strings, so
 * every icon is created element by element. The nodes work in plain DOM
 * (`append`) and in Lit templates (`${icon('copy')}`). Every icon is decorative
 * (`aria-hidden`): the button or text next to it carries the meaning. Colour
 * and stroke come from `.cb-icon` in base.css (`currentColor`).
 */

const SVG_NS = 'http://www.w3.org/2000/svg';

/** One shape of a drawing: tag and attributes. `cb-icon-solid` fills it. */
export type Shape = readonly [tag: 'path' | 'rect' | 'circle', attributes: Record<string, string>];

const ICONS = {
    copy: [
        ['rect', { x: '5.5', y: '5.5', width: '8', height: '8', rx: '1.5' }],
        ['path', { d: 'M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2' }],
    ],
    check: [['path', { d: 'm3.5 8.5 3 3 6-7' }]],
    download: [['path', { d: 'M8 2.5v8m0 0L4.8 7.3M8 10.5l3.2-3.2M3 13.5h10' }]],
    'chevron-left': [['path', { d: 'M10 3.5 5.5 8l4.5 4.5' }]],
    'chevron-right': [['path', { d: 'M6 3.5 10.5 8 6 12.5' }]],
    'chevron-down': [['path', { d: 'M3.5 6 8 10.5 12.5 6' }]],
    remove: [['path', { d: 'm4.5 4.5 7 7m0-7-7 7' }]],
    lock: [
        ['rect', { x: '3.5', y: '7', width: '9', height: '6.5', rx: '1.5' }],
        ['path', { d: 'M5.5 7V5a2.5 2.5 0 0 1 5 0v2' }],
    ],
    clock: [
        ['circle', { cx: '8', cy: '8', r: '5.8' }],
        ['path', { d: 'M8 4.8V8l2 1.3' }],
    ],
    stack: [
        [
            'path',
            { d: 'M8 2.5 14 5.5 8 8.5 2 5.5l6-3ZM2.5 8.5 8 11.3l5.5-2.8M2.5 11 8 13.8 13.5 11' },
        ],
    ],
    theme: [
        ['circle', { cx: '8', cy: '8', r: '5.8' }],
        ['path', { d: 'M8 2.2v11.6A5.8 5.8 0 0 0 8 2.2Z', class: 'cb-icon-solid' }],
    ],
    sun: [
        ['circle', { cx: '8', cy: '8', r: '3' }],
        [
            'path',
            {
                d: 'M8 1.5V3m0 10v1.5M1.5 8H3m10 0h1.5M3.4 3.4l1.06 1.06m7.08 7.08 1.06 1.06M3.4 12.6l1.06-1.06m7.08-7.08 1.06-1.06',
            },
        ],
    ],
    moon: [['path', { d: 'M13 9.5A5.5 5.5 0 1 1 6.5 3a4.5 4.5 0 0 0 6.5 6.5Z' }]],
} satisfies Record<string, readonly Shape[]>;

export type IconName = keyof typeof ICONS;

/** An `<svg>` with `viewBox`, `class` and the given shapes, hidden from assistive technology. */
export function drawing(
    viewBox: string,
    className: string,
    shapes: readonly Shape[],
): SVGSVGElement {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('class', className);
    svg.setAttribute('viewBox', viewBox);
    svg.setAttribute('aria-hidden', 'true');
    svg.setAttribute('focusable', 'false');
    for (const [tag, attributes] of shapes) {
        const shape = document.createElementNS(SVG_NS, tag);
        for (const [name, value] of Object.entries(attributes)) shape.setAttribute(name, value);
        svg.append(shape);
    }
    return svg;
}

/** The icon `name`, 16 × 16 units, marked with `data-icon`. */
export function icon(name: IconName): SVGSVGElement {
    const svg = drawing('0 0 16 16', 'cb-icon', ICONS[name]);
    svg.dataset.icon = name;
    return svg;
}
