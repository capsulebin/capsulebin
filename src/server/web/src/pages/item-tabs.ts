/**
 * Shared behaviour of the item tab bar on the create and the view page.
 *
 * Both pages render their own markup (Lit on the view page, plain DOM on the
 * create page) but follow the WAI-ARIA tabs pattern the same way: a
 * `role="tablist"` of `role="tab"` buttons with a roving tabindex, arrow keys
 * move between tabs, Home/End jump to the ends.
 */

/** The tab index a key press moves to, or `null` if the key is not ours. */
export function nextTabIndex(key: string, current: number, count: number): number | null {
    if (count <= 0) return null;
    switch (key) {
        case 'ArrowRight':
            return (current + 1) % count;
        case 'ArrowLeft':
            return (current - 1 + count) % count;
        case 'Home':
            return 0;
        case 'End':
            return count - 1;
        default:
            return null;
    }
}

/**
 * After switching items: if the page is scrolled past the start of the
 * content, jump to the start of the new item, just below the sticky header.
 */
export function revealActiveItem(header: HTMLElement | null, panel: HTMLElement | null): void {
    if (!header || !panel) return;
    const headerBottom = header.getBoundingClientRect().bottom;
    const panelTop = panel.getBoundingClientRect().top;
    // Only when the content start is hidden behind (or above) the sticky header;
    // at the top of the page nothing needs to move.
    if (panelTop < headerBottom) {
        window.scrollBy({ top: panelTop - headerBottom - 8 });
    }
}

/**
 * Fits the tabs of `bar` into its width: tabs that do not fit are hidden and
 * their indices returned, for the page to list in its "N more" menu. The active
 * tab is always kept visible - if it would overflow, it takes the last visible
 * place. `more` is the menu button next to the bar; it is shown only when
 * something overflows.
 *
 * Works on the bar's direct children (tab buttons on the view page, tab
 * wrappers with a close button on the create page).
 */
export function fitTabs(bar: HTMLElement, more: HTMLElement, active: number): number[] {
    const tabs = [...bar.children] as HTMLElement[];
    for (const tab of tabs) tab.hidden = false;
    more.hidden = true;

    const available = bar.clientWidth;
    const gap = Number.parseFloat(getComputedStyle(bar).columnGap) || 0;
    const widths = tabs.map((tab) => tab.getBoundingClientRect().width);
    const total =
        widths.reduce((sum, width) => sum + width, 0) + gap * Math.max(0, tabs.length - 1);
    // No layout (jsdom, a hidden page): show everything rather than guess.
    if (available === 0 || total <= available) return [];

    more.hidden = false;
    const moreWidth = more.getBoundingClientRect().width + gap;

    const visible = new Set<number>([active]);
    let used = (widths[active] ?? 0) + moreWidth;
    for (const [index, width] of widths.entries()) {
        if (index === active) continue;
        if (used + width + gap > available) break;
        visible.add(index);
        used += width + gap;
    }

    const hidden: number[] = [];
    for (const [index, tab] of tabs.entries()) {
        tab.hidden = !visible.has(index);
        if (tab.hidden) hidden.push(index);
    }
    return hidden;
}
