import type { Locator, Page } from '@playwright/test';
import { expect } from './guard';

/**
 * Computed styles as the engine resolves them, shared by the layout, items and
 * state specs. Colours are compared in one canonical form: a canvas context
 * normalises any CSS colour (`rgb(…)`, `#hex`) to `#rrggbb`, or `rgba(…)` for a
 * translucent one, the same way in every engine.
 */

/** The value of the colour token `name` (`--cb-accent`) on the page, normalised. */
export async function token(page: Page, name: string): Promise<string> {
    return page.evaluate((name) => {
        const context = document.createElement('canvas').getContext('2d')!;
        context.fillStyle = getComputedStyle(document.documentElement)
            .getPropertyValue(name)
            .trim();
        return context.fillStyle;
    }, name);
}

/** The computed colour `property` (`color`, `backgroundColor`, `stroke`, `fill`) of the element, normalised. */
export async function colour(
    locator: Locator,
    property: 'color' | 'backgroundColor' | 'stroke' | 'fill',
): Promise<string> {
    return locator.evaluate((element, property) => {
        const context = document.createElement('canvas').getContext('2d')!;
        context.fillStyle = getComputedStyle(element)[property];
        return context.fillStyle;
    }, property);
}

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
    const [r, g, b] = [1, 3, 5].map((start) => {
        const c = Number.parseInt(hex.slice(start, start + 2), 16) / 255;
        return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
    }) as [number, number, number];
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio of two `#rrggbb` colours, as `token()` and `colour()` return them. */
export function contrast(a: string, b: string): number {
    const [low, high] = [luminance(a), luminance(b)].sort((x, y) => x - y) as [number, number];
    return (high + 0.05) / (low + 0.05);
}

/**
 * Every rendered element under `root` (the root included) whose computed
 * `property` equals `value` (normalised), except those inside an element
 * matching `except`, as a short description each: tag, classes and data
 * attributes, enough to name the offender in a failure.
 */
export async function elementsWith(
    root: Locator,
    property: 'color' | 'backgroundColor',
    value: string,
    except = ':not(*)',
): Promise<string[]> {
    return root.evaluate(
        (root, [property, value, except]) => {
            const context = document.createElement('canvas').getContext('2d')!;
            const normalise = (colour: string) => {
                context.fillStyle = '#000000';
                context.fillStyle = colour;
                return context.fillStyle;
            };
            return [root, ...root.querySelectorAll('*')]
                .filter((element) => {
                    const box = element.getBoundingClientRect();
                    return box.width > 0 && box.height > 0;
                })
                .filter((element) => !element.closest(except))
                .filter((element) => normalise(getComputedStyle(element)[property]) === value)
                .map((element) =>
                    [
                        element.localName,
                        ...[...element.classList].map((name) => `.${name}`),
                        ...[...element.attributes]
                            .filter((attribute) => attribute.name.startsWith('data-'))
                            .map((attribute) => `[${attribute.name}]`),
                    ].join(''),
                );
        },
        [property, value, except] as const,
    );
}

/** Px of a computed length such as `8px`. */
export const px = (value: string): number => Number.parseFloat(value);

/**
 * The duration of `property`'s transition on the element in seconds, from the
 * parallel lists `transition-property` and `transition-duration` (`all` counts);
 * `null` if the property has no transition.
 */
export async function transitionOf(locator: Locator, property: string): Promise<number | null> {
    return locator.evaluate((element, property) => {
        const style = getComputedStyle(element);
        const properties = style.transitionProperty.split(',').map((p) => p.trim());
        const durations = style.transitionDuration.split(',').map((d) => d.trim());
        const index = properties.findIndex((p) => p === property || p === 'all');
        if (index < 0) return null;
        // The duration list repeats when it is shorter than the property list.
        return Number.parseFloat(durations[index % durations.length]!);
    }, property);
}

/**
 * Waits until every finite animation of the page has ended: the entry animation
 * moves elements (`translateY`), which a box measured meanwhile would include.
 */
export async function settled(page: Page): Promise<void> {
    await page.evaluate(() =>
        Promise.all(
            document
                .getAnimations()
                .filter(
                    (animation) => animation.effect?.getComputedTiming().iterations !== Infinity,
                )
                .map((animation) => animation.finished.catch(() => undefined)),
        ),
    );
}

/** The element's box once the page's entry animation has ended. */
export async function box(
    locator: Locator,
): Promise<{ x: number; y: number; width: number; height: number }> {
    await settled(locator.page());
    const found = await locator.boundingBox();
    expect(found, 'the element has a box').not.toBeNull();
    return found!;
}

export async function expectNoHorizontalScroll(page: Page): Promise<void> {
    const width = page.viewportSize()!.width;
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
    );
}
