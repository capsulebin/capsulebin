/**
 * Recipient view: decryption and display of a single capsule.
 *
 * The key arrives in the URL fragment (`#v1.<base64url>`) and is never sent
 * to the server. The flow:
 *
 *   1. Parse the fragment → link secret.
 *   2. GET the descriptor → public header, lifecycle metadata.
 *   3. Derive the reveal token from the link secret and the header's HKDF salt.
 *   4. POST the reveal token → ciphertext (or 410 if burned).
 *   5. Decrypt the ciphertext with the content key derived from the link secret.
 *   6. Decode the capsule and display the items.
 *
 * Every error path produces a generic message so that an observer cannot
 * distinguish "expired" from "wrong key" from "never existed".
 *
 * Rendered as a light-DOM `LitElement` (`createRenderRoot()` returns `this`):
 * no shadow root, so the page's global `base.css`/`tokens.css` design tokens
 * keep applying unchanged, and Lit's Shadow-DOM styling path (which relies on
 * `adoptedStyleSheets`, falling back to injected `<style>` tags on browsers
 * below Safari 16.4/Firefox 101) never comes into play.
 */

import { LitElement, html, type PropertyValues } from 'lit';

import {
    derivePasswordKey,
    fromBase64url,
    type PublicHeader,
    pasteIdFromInput,
} from '@capsulebin/core';
import { formatExpiry, formatSize, localPath, t, tn } from '../language';
import { byAttr, initShell, isLeavingPage, languageFromNextPage } from './shell';
import { openCapsule, type PasswordRetry } from './open-capsule';
import { fitTabs, nextTabIndex, revealActiveItem } from './item-tabs';
import { CopyFeedback, createLiveRegion } from './copy-state';
import { type HexagonState, hexagon } from './hexagon';
import { type IconName, icon } from './icons';
import {
    downloadItem,
    itemName,
    itemMeta,
    kindBadge,
    renderItemContent,
    toDisplayItems,
    type DisplayItem,
} from './render-items';

// --- component -----------------------------------------------------------------

type Phase =
    'loading' | 'decrypting' | 'password' | 'burn-warning' | 'display' | 'burned' | 'error';

interface PasswordContext {
    pwdSalt: Uint8Array;
    hkdfSalt: Uint8Array;
    iterations: number;
}

class CbViewPage extends LitElement {
    static override properties = {
        phase: {},
        errorMessage: {},
        passwordError: {},
        passwordBusy: {},
        kdfProgressLabel: {},
        capsuleTitle: {},
        items: {},
        activeIndex: {},
        sourceView: {},
        menuOpen: {},
        expiresAt: {},
        burnAfterRetrieval: {},
    };

    // `declare`-only: these must not emit constructor field initializers, which
    // would shadow the reactive accessors `static properties` installs on the
    // prototype (the well-known Lit + TypeScript class-fields caveat).
    declare phase: Phase;
    declare errorMessage: string;
    declare passwordError: string;
    declare passwordBusy: boolean;
    declare kdfProgressLabel: string;
    declare capsuleTitle: string | null;
    declare items: DisplayItem[];
    declare activeIndex: number;
    /** Markdown items currently shown as source instead of rendered. */
    declare sourceView: ReadonlySet<number>;
    /** The "All N" list is open. */
    declare menuOpen: boolean;
    declare expiresAt: number | null;
    /** From the descriptor: this retrieval deleted the capsule. */
    declare burnAfterRetrieval: boolean;

    #pwdContext: PasswordContext | null = null;
    #resolvePassword: ((key: Uint8Array) => void) | null = null;
    #resolveBurnConfirm: (() => void) | null = null;
    /** Announces a copy; rendered with the display so it is in place before it speaks. */
    #liveRegion = createLiveRegion();
    #copyFeedback = new CopyFeedback(this.#liveRegion);
    #resizeObserver: ResizeObserver | undefined;
    /**
     * Decorative SVG nodes, built once per place and state or icon: given the
     * same node again, Lit leaves it in the DOM, so a re-render keeps a running
     * hexagon's animation going. Keyed by place, as a node has one parent.
     */
    #drawings = new Map<string, SVGSVGElement>();
    #onDocumentClick = (event: MouseEvent): void => {
        const target = event.target as Node;
        if (
            this.menuOpen &&
            !this.#allButton()?.contains(target) &&
            !this.#menu()?.contains(target)
        ) {
            this.menuOpen = false;
        }
    };
    #onDocumentKey = (event: KeyboardEvent): void => {
        if (event.key === 'Escape' && this.menuOpen) {
            this.menuOpen = false;
            // After the update, like #chooseFromMenu: focusing while the menu is
            // still rendered open lets Chromium move focus again on the re-render.
            void this.updateComplete.then(() => this.#allButton()?.focus());
        }
    };

    constructor() {
        super();
        this.phase = 'loading';
        this.errorMessage = '';
        this.passwordError = '';
        this.passwordBusy = false;
        this.kdfProgressLabel = '';
        this.capsuleTitle = null;
        this.items = [];
        this.activeIndex = 0;
        this.sourceView = new Set();
        this.menuOpen = false;
        this.expiresAt = null;
        this.burnAfterRetrieval = false;
    }

    override createRenderRoot(): this {
        return this;
    }

    override connectedCallback(): void {
        super.connectedCallback();
        document.addEventListener('click', this.#onDocumentClick);
        document.addEventListener('keydown', this.#onDocumentKey);
        this.#openCapsule();
    }

    override disconnectedCallback(): void {
        super.disconnectedCallback();
        this.#copyFeedback.restore();
        this.#resizeObserver?.disconnect();
        document.removeEventListener('click', this.#onDocumentClick);
        document.removeEventListener('keydown', this.#onDocumentKey);
    }

    protected override updated(changed: PropertyValues): void {
        if (changed.has('phase')) {
            // Plays the same fade-in the old template-clone code triggered on every
            // phase transition, and only on phase transitions — switching the
            // active item below does not replay it.
            this.classList.add('cb-enter');
            this.addEventListener('animationend', () => this.classList.remove('cb-enter'), {
                once: true,
            });

            if (this.phase === 'display') {
                // Unlike the other phases, decrypted content needs the full width of
                // .cb-main instead of the narrow centered column used elsewhere.
                byAttr<HTMLElement>('view-root').classList.remove('cb-center');
            }
        }

        this.#fitTabs();

        if (changed.has('activeIndex') && changed.get('activeIndex') !== undefined) {
            revealActiveItem(
                this.querySelector<HTMLElement>('[data-sheet] .cb-sheet-bar'),
                this.querySelector<HTMLElement>('.cb-item-panel'),
            );
        }
    }

    protected override render() {
        switch (this.phase) {
            case 'loading':
                return this.#renderLoading();
            case 'decrypting':
                return this.#renderDecrypting();
            case 'password':
                return this.#renderPasswordPrompt();
            case 'burn-warning':
                return this.#renderBurnWarning();
            case 'display':
                return this.#renderDisplay();
            case 'burned':
                return this.#renderBurned();
            case 'error':
                return this.#renderError();
        }
    }

    #renderLoading() {
        return html`
            ${this.#hexagon('state', 'running')}
            <h1 class="cb-h1">${t('view.opening')}</h1>
            <p class="cb-lede">${t('view.openingLede')}</p>
        `;
    }

    #renderDecrypting() {
        return html`
            ${this.#hexagon('state', 'running')}
            <h1 class="cb-h1">${t('view.decrypting')}</h1>
            <p class="cb-lede">${t('view.decryptingLede')}</p>
        `;
    }

    #renderBurned() {
        return html`
            ${this.#hexagon('state', 'empty')}
            <h1 class="cb-h1">${t('view.burned')}</h1>
            <p class="cb-lede">${t('view.burnedLede')}</p>
            <p><a href=${localPath('/')} class="cb-btn">${t('view.createOwn')}</a></p>
        `;
    }

    #renderBurnWarning() {
        return html`
            ${this.#hexagon('state', 'warning')}
            <h1 class="cb-h1">${t('view.burnWarning')}</h1>
            <p class="cb-lede">${t('view.burnWarningLede')}</p>
            <div class="cb-callout cb-callout-danger">
                <strong>${t('view.cannotUndo')}</strong>
                ${t('view.burnWarningDetail')}
            </div>
            <p>
                <button type="button" class="cb-btn" @click=${() => this.#confirmBurn()}>
                    ${t('view.burnConfirm')}
                </button>
                <a href=${localPath('/')} class="cb-btn cb-btn-ghost">${t('view.cancel')}</a>
            </p>
        `;
    }

    #renderPasswordPrompt() {
        return html`
            ${this.#hexagon('state', 'locked')}
            <h1 class="cb-h1">${t('view.passwordTitle')}</h1>
            <p class="cb-lede">${t('view.passwordLede')}</p>
            <form class="cb-form" @submit=${(e: SubmitEvent) => this.#handlePasswordSubmit(e)}>
                <label class="cb-label" for="pwd-input">${t('view.password')}</label>
                <input
                    type="password"
                    id="pwd-input"
                    class="cb-input"
                    placeholder=${t('view.passwordPlaceholder')}
                    autocomplete="off"
                    required
                    .disabled=${this.passwordBusy}
                />
                <p class="cb-error" ?hidden=${!this.passwordError}>${this.passwordError}</p>
                <div class="cb-kdf-progress" ?hidden=${!this.passwordBusy}>
                    ${this.passwordBusy ? this.#hexagon('kdf', 'running') : ''}
                    <p class="cb-lede">${this.kdfProgressLabel}</p>
                </div>
                <button type="submit" class="cb-btn" .disabled=${this.passwordBusy}>
                    ${t('view.decrypt')}
                </button>
            </form>
        `;
    }

    #renderError() {
        return html`
            ${this.#hexagon('state', 'empty')}
            <h1 class="cb-h1">${t('view.unavailable')}</h1>
            <p class="cb-lede">${this.errorMessage || t('view.unavailableLede')}</p>
            <p><a href=${localPath('/')} class="cb-btn">${t('view.createOwn')}</a></p>
        `;
    }

    /**
     * The decrypted capsule: title, chip row, and one sheet holding the tab bar
     * (several items), the active item's actions, its content and (several
     * items) the previous/next row.
     */
    #renderDisplay() {
        const items = this.items;
        const index = Math.min(this.activeIndex, items.length - 1);
        const active = items[index];
        const multi = items.length > 1;

        return html`
            <div class="cb-view-head">
                ${
                    this.capsuleTitle
                        ? html`<h1 class="cb-h1">${this.capsuleTitle}</h1>`
                        : html`<h1 class="cb-sr-only">${t('view.capsule')}</h1>`
                }
                ${this.#renderChips(items)}
            </div>
            ${
                active
                    ? html`
                      <div class="cb-sheet cb-view-sheet" data-sheet>
                          <div class="cb-sheet-bar">
                              ${multi ? this.#renderTabs(items, index) : this.#renderItemName(active, index)}
                              ${this.#renderItemActions(active, index)}
                              ${multi ? this.#renderAllMenu(items, index) : ''}
                          </div>
                          <div
                              class="cb-item-panel"
                              role=${multi ? 'tabpanel' : 'region'}
                              id="cb-item-panel"
                              aria-labelledby=${multi ? `cb-item-tab-${index}` : 'cb-item-name'}
                          >
                              ${renderItemContent(active, {
                                  source: this.sourceView.has(index),
                                  index,
                              })}
                          </div>
                          ${multi ? this.#renderItemNav(items, index) : ''}
                      </div>
                  `
                    : ''
            }
            ${this.#liveRegion}
        `;
    }

    /** Lifecycle and size under the title; a burn capsule's warning comes first. */
    #renderChips(items: DisplayItem[]) {
        const size = formatSize(items.reduce((sum, item) => sum + (item.bytes?.length ?? 0), 0));
        const amount = items.length > 1 ? `${tn('view.itemCount', items.length)} · ${size}` : size;
        return html`
            <ul class="cb-chips" data-chips>
                ${
                    this.burnAfterRetrieval
                        ? html`<li class="cb-chip cb-chip-warning" data-chip="burn">
                          ${t('view.deletedAfterRetrieval')}
                      </li>`
                        : ''
                }
                <li class="cb-chip" data-chip="expiry">${this.#icon('chip', 'clock')}${formatExpiry(this.expiresAt)}</li>
                <li class="cb-chip" data-chip="size">${this.#icon('chip', 'stack')}${amount}</li>
                <li class="cb-chip" data-chip="local">${this.#icon('chip', 'lock')}${t('view.decryptedLocally')}</li>
            </ul>
        `;
    }

    #renderItemName(item: DisplayItem, index: number) {
        return html`
            <span class="cb-view-item">
                <span class="cb-itembar-name" id="cb-item-name">${itemName(item, index)}</span>
                <span class="cb-itembar-meta">${itemMeta(item)}</span>
            </span>
        `;
    }

    #renderTabs(items: DisplayItem[], active: number) {
        return html`
            <div
                class="cb-itemtabs"
                role="tablist"
                aria-label=${t('view.items')}
                @keydown=${(e: KeyboardEvent) => this.#onTabKey(e)}
            >
                ${items.map(
                    (item, i) => html`
                        <button
                            type="button"
                            role="tab"
                            class="cb-itemtab"
                            id="cb-item-tab-${i}"
                            aria-selected=${i === active ? 'true' : 'false'}
                            aria-controls="cb-item-panel"
                            tabindex=${i === active ? 0 : -1}
                            title=${`${itemName(item, i)} · ${itemMeta(item)}`}
                            @click=${() => this.#select(i)}
                        >
                            <span class="cb-itemtab-kind">${kindBadge(item)}</span>
                            <span class="cb-itemtab-name">${itemName(item, i)}</span>
                        </button>
                    `,
                )}
            </div>
            <button
                type="button"
                class="cb-quiet cb-all-btn"
                data-items-all
                hidden
                aria-haspopup="menu"
                aria-expanded=${this.menuOpen ? 'true' : 'false'}
                aria-controls="cb-all-menu"
                @click=${() => this.#toggleMenu()}
            >
                <span>${t('items.all', { count: items.length })}</span>${this.#icon('all', 'chevron-down')}
            </button>
        `;
    }

    /** Every item in manifest order, the same list whatever the width hides. */
    #renderAllMenu(items: DisplayItem[], active: number) {
        return html`
            <div
                id="cb-all-menu"
                class="cb-menu cb-all-menu"
                role="menu"
                aria-label=${t('view.items')}
                ?hidden=${!this.menuOpen}
                @keydown=${(e: KeyboardEvent) => this.#onMenuKey(e)}
            >
                ${items.map(
                    (item, i) => html`
                        <button
                            type="button"
                            class="cb-menu-row"
                            role="menuitemradio"
                            tabindex="-1"
                            aria-checked=${i === active ? 'true' : 'false'}
                            @click=${() => this.#chooseFromMenu(i)}
                        >
                            <span class="cb-itemtab-kind">${kindBadge(item)}</span>
                            <span class="cb-itemtab-name">${itemName(item, i)}</span>
                            <span class="cb-menu-size">${formatSize(item.bytes?.length ?? 0)}</span>
                        </button>
                    `,
                )}
            </div>
        `;
    }

    /** Previous/next below the content, for whoever has read an item to its end. */
    #renderItemNav(items: DisplayItem[], index: number) {
        const previous = index > 0 ? itemName(items[index - 1]!, index - 1) : '';
        const next = index < items.length - 1 ? itemName(items[index + 1]!, index + 1) : '';
        return html`
            <div class="cb-item-nav">
                <button
                    type="button"
                    class="cb-quiet"
                    data-item-previous
                    aria-label=${t('items.previous')}
                    ?disabled=${index === 0}
                    @click=${() => this.#select(index - 1)}
                >
                    ${this.#icon('nav', 'chevron-left')}<span class="cb-bar-label">${previous}</span>
                </button>
                <span class="cb-item-nav-position">
                    ${t('items.position', { index: index + 1, count: items.length })}
                </span>
                <button
                    type="button"
                    class="cb-quiet"
                    data-item-next
                    aria-label=${t('items.next')}
                    ?disabled=${index === items.length - 1}
                    @click=${() => this.#select(index + 1)}
                >
                    <span class="cb-bar-label">${next}</span>${this.#icon('nav', 'chevron-right')}
                </button>
            </div>
        `;
    }

    /** Hides the tabs that do not fit and shows the "All N" button if any do not.
     *  Runs after every render and on resize. */
    #fitTabs(): void {
        const row = this.querySelector<HTMLElement>('[data-sheet] .cb-sheet-bar');
        const bar = row?.querySelector<HTMLElement>('[role="tablist"]');
        const all = this.#allButton();
        if (!row || !bar || !all) return;

        if (!this.#resizeObserver) {
            this.#resizeObserver = new ResizeObserver(() => this.#fitTabs());
            this.#resizeObserver.observe(row);
        }

        const hidden = fitTabs(bar, all, this.activeIndex);
        if (hidden.length === 0 && this.menuOpen) this.menuOpen = false;
    }

    #renderItemActions(item: DisplayItem, index: number) {
        const isMarkdown = item.presentation === 'markdown';
        const isFile = item.presentation === 'download';
        const source = this.sourceView.has(index);

        return html`
            <div class="cb-bar-actions">
                ${
                    isMarkdown
                        ? html`
                          <span class="cb-view-switch" role="group" aria-label=${t('view.markdownView')}>
                              <button
                                  type="button"
                                  aria-pressed=${source ? 'false' : 'true'}
                                  @click=${() => this.#setSource(index, false)}
                              >
                                  ${t('view.preview')}
                              </button>
                              <button
                                  type="button"
                                  aria-pressed=${source ? 'true' : 'false'}
                                  @click=${() => this.#setSource(index, true)}
                              >
                                  ${t('view.source')}
                              </button>
                          </span>
                      `
                        : ''
                }
                ${
                    isFile
                        ? ''
                        : html`
                          <button
                              type="button"
                              class="cb-quiet"
                              data-item-copy
                              @click=${(e: MouseEvent) =>
                                  this.#copy(e.currentTarget as HTMLButtonElement, item)}
                          >
                              ${this.#icon('action', 'copy')}<span class="cb-bar-label">${t('view.copy')}</span>
                          </button>
                      `
                }
                <button
                    type="button"
                    class="cb-quiet"
                    data-item-download
                    @click=${() => downloadItem(item, index)}
                >
                    ${this.#icon('action', 'download')}<span class="cb-bar-label">${t('view.download')}</span>
                </button>
            </div>
        `;
    }

    /** The hexagon for `state` at `place`, the same node on every render. */
    #hexagon(place: string, state: HexagonState): SVGSVGElement {
        return this.#drawing(`hexagon:${place}:${state}`, () => hexagon(state));
    }

    /** The icon `name` at `place`, the same node on every render. */
    #icon(place: string, name: IconName): SVGSVGElement {
        return this.#drawing(`icon:${place}:${name}`, () => icon(name));
    }

    #drawing(key: string, build: () => SVGSVGElement): SVGSVGElement {
        let node = this.#drawings.get(key);
        if (!node) {
            node = build();
            this.#drawings.set(key, node);
        }
        return node;
    }

    #allButton(): HTMLButtonElement | null {
        return this.querySelector<HTMLButtonElement>('[data-items-all]');
    }

    #menu(): HTMLElement | null {
        return this.querySelector<HTMLElement>('#cb-all-menu');
    }

    #toggleMenu(): void {
        this.menuOpen = !this.menuOpen;
        if (this.menuOpen) {
            void this.updateComplete.then(() =>
                this.#menu()?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus(),
            );
        }
    }

    /** A row of the "All N" list: the item becomes the active (visible) tab. */
    #chooseFromMenu(index: number): void {
        this.menuOpen = false;
        this.#select(index);
        void this.updateComplete.then(() =>
            this.querySelector<HTMLElement>(`#cb-item-tab-${index}`)?.focus(),
        );
    }

    #select(index: number): void {
        if (index === this.activeIndex || index < 0 || index >= this.items.length) return;
        // The copy button stays in place for the next item; its "Copied" was for this one.
        this.#copyFeedback.restore();
        this.activeIndex = index;
    }

    #onTabKey(e: KeyboardEvent): void {
        const next = nextTabIndex(e.key, this.activeIndex, this.items.length);
        if (next === null) return;
        e.preventDefault();
        this.#select(next);
        void this.updateComplete.then(() =>
            this.querySelector<HTMLElement>(`#cb-item-tab-${next}`)?.focus(),
        );
    }

    #onMenuKey(e: KeyboardEvent): void {
        const options = [
            ...(this.#menu()?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? []),
        ];
        const index = options.indexOf(document.activeElement as HTMLElement);
        const key =
            e.key === 'ArrowDown' ? 'ArrowRight' : e.key === 'ArrowUp' ? 'ArrowLeft' : e.key;
        const next = nextTabIndex(key, index, options.length);
        if (next === null) return;
        e.preventDefault();
        options[next]!.focus();
    }

    #setSource(index: number, source: boolean): void {
        const next = new Set(this.sourceView);
        if (source) next.add(index);
        else next.delete(index);
        this.sourceView = next;
    }

    /** Copies the item's text; a refused clipboard leaves the button as it is
     *  (selecting the text by hand still works). */
    #copy(button: HTMLButtonElement, item: DisplayItem): void {
        void this.#copyFeedback.copy(button, item.text);
    }

    #confirmBurn(): void {
        if (!startOpening()) return;
        this.#resolveBurnConfirm?.();
        this.#resolveBurnConfirm = null;
    }

    #awaitBurnConfirm(): Promise<void> {
        this.phase = 'burn-warning';
        return new Promise((resolve) => {
            this.#resolveBurnConfirm = resolve;
        });
    }

    #showError(message: string): void {
        this.errorMessage = message;
        this.phase = 'error';
    }

    /**
     * Shows the password prompt and returns the derived key on success.
     *
     * There is no local pre-check: a wrong password derives a wrong reveal
     * token, and the server rejects it exactly like a wrong link — the same
     * generic error, discovered only via the rate-limited retrieve request.
     * A local verifier would have meant storing it in the public header,
     * which anyone who knows the paste id can fetch unauthenticated —
     * turning it into an offline brute-force oracle. See ADR-001.
     *
     * When the server refused the previous answer, the prompt comes back with
     * `retry`. The describe token was accepted, so a wrong password is the
     * likely cause, but the capsule may also have gone away in the meantime;
     * the server answers both alike, and so does the message.
     */
    #promptForPassword(header: PublicHeader, retry?: PasswordRetry): Promise<Uint8Array> {
        const pwd = header.password!;
        this.#pwdContext = {
            pwdSalt: fromBase64url(pwd.salt),
            hkdfSalt: header.hkdfSaltBytes(),
            iterations: pwd.iterations,
        };
        this.passwordError = passwordRetryMessage(retry);
        this.phase = 'password';

        return new Promise((resolve) => {
            this.#resolvePassword = resolve;
        });
    }

    async #handlePasswordSubmit(e: SubmitEvent): Promise<void> {
        e.preventDefault();
        const form = e.currentTarget as HTMLFormElement;
        const input = form.querySelector<HTMLInputElement>('#pwd-input')!;
        const password = input.value;
        if (!password || !this.#pwdContext || !startOpening()) return;

        const { pwdSalt, iterations } = this.#pwdContext;

        this.passwordBusy = true;
        this.kdfProgressLabel = tn('view.derivingKey', iterations);

        try {
            const pwdKey = await derivePasswordKey(password, pwdSalt, iterations);
            this.#resolvePassword?.(pwdKey);
        } catch {
            this.passwordError = t('error.generic');
        } finally {
            this.passwordBusy = false;
        }
    }

    // --- flow --------------------------------------------------------------------

    async #openCapsule(): Promise<void> {
        const outcome = await openCapsule({
            onDecrypting: () => {
                this.phase = 'decrypting';
            },
            // A capsule without burn warning or password gets here without a click.
            beforeRetrieve: startOpening,
            confirmBurn: () => this.#awaitBurnConfirm(),
            promptForPassword: (header, retry) => this.#promptForPassword(header, retry),
        });

        switch (outcome.kind) {
            case 'success':
                this.capsuleTitle = outcome.title;
                this.items = toDisplayItems(outcome.items);
                this.activeIndex = 0;
                this.expiresAt = outcome.expiresAt;
                this.burnAfterRetrieval = outcome.burnAfterRetrieval;
                this.phase = 'display';
                forgetKey();
                break;
            case 'burned':
                this.phase = 'burned';
                forgetKey();
                break;
            case 'error':
                this.#showError(outcome.message);
                if (outcome.final) forgetKey();
                break;
            case 'aborted':
                // The password hook already left the UI in a terminal state, or the
                // user navigated away — nothing further to do.
                break;
        }
    }
}

/** The prompt's message after the server refused a password, empty on the first ask. */
function passwordRetryMessage(retry: PasswordRetry | undefined): string {
    if (!retry) return '';
    if (retry.reason === 'rejected') return t('view.wrongPassword');
    const wait = retry.retryAfter;
    return wait ? tn('view.tooManyAttemptsFor', wait) : t('view.tooManyAttempts');
}

/**
 * The go-ahead for the retrieval - the user's (burn confirmation, password) and
 * once more right before the request, which a capsule without either reaches on
 * its own: refused while the language switch is loading this page in another
 * language - the capsule would be retrieved or burned here and shown nowhere,
 * and the new page would retrieve it again. Otherwise the switch only stores
 * the choice from here on: after a click not only from the request that follows
 * the key derivation.
 */
function startOpening(): boolean {
    if (isLeavingPage(shell)) return false;
    languageFromNextPage(shell);
    return true;
}

/**
 * Removes the key (the fragment) from the address bar and the current history
 * entry, once the link has served its purpose or cannot work any more - so it
 * does not linger on screen, in screenshots or in a shared screen. Reloading the
 * page in another language would then lose the key, so the language switch only
 * stores the choice from here on (if it does not already, since the retrieval).
 */
function forgetKey(): void {
    history.replaceState(null, '', window.location.pathname);
    languageFromNextPage(shell);
}

// --- boot --------------------------------------------------------------------

const shell = initShell();
customElements.define('cb-view-page', CbViewPage);

// Point the footer's report link at this capsule (`/report?id=…`): only the id
// from the path, never anything from the fragment.
{
    const id = pasteIdFromInput(window.location.pathname);
    const report = localPath('/report');
    const link = document.querySelector<HTMLAnchorElement>(`a[href="${report}"]`);
    if (id && link) link.href = `${report}?id=${id}`;
}
