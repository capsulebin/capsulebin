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
    formatExpiry,
    formatSize,
    pasteIdFromInput,
} from '@capsulebin/core';
import { byAttr, initShell } from './shell';
import { openCapsule, type PasswordRetry } from './open-capsule';
import { fitTabs, nextTabIndex, revealActiveItem } from './item-tabs';
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
        copiedIndex: {},
        overflow: {},
        moreOpen: {},
        expiresAt: {},
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
    /** The item whose content was just copied, for the button's feedback. */
    declare copiedIndex: number | null;
    /** Items whose tabs do not fit in the bar (counted on the "N more" button). */
    declare overflow: readonly number[];
    declare moreOpen: boolean;
    declare expiresAt: number | null;

    #pwdContext: PasswordContext | null = null;
    #resolvePassword: ((key: Uint8Array) => void) | null = null;
    #resolveBurnConfirm: (() => void) | null = null;
    #copiedTimer: ReturnType<typeof setTimeout> | undefined;
    #resizeObserver: ResizeObserver | undefined;
    #onDocumentClick = (event: MouseEvent): void => {
        if (this.moreOpen && !this.querySelector('.cb-more')?.contains(event.target as Node)) {
            this.moreOpen = false;
        }
    };
    #onDocumentKey = (event: KeyboardEvent): void => {
        if (event.key === 'Escape' && this.moreOpen) {
            this.moreOpen = false;
            this.querySelector<HTMLElement>('.cb-more-btn')?.focus();
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
        this.copiedIndex = null;
        this.overflow = [];
        this.moreOpen = false;
        this.expiresAt = null;
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
        clearTimeout(this.#copiedTimer);
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
                this.querySelector<HTMLElement>('.cb-viewer-head'),
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
            <h1 class="cb-h1">Opening capsule&hellip;</h1>
            <p class="cb-lede">
                The key is in the fragment of this address and does not leave your browser.
            </p>
        `;
    }

    #renderDecrypting() {
        return html`
            <h1 class="cb-h1">Decrypting&hellip;</h1>
            <div class="cb-spinner"></div>
            <p class="cb-lede cb-pulse">
                Decrypting in your browser. The server cannot read this content.
            </p>
        `;
    }

    #renderBurned() {
        return html`
            <h1 class="cb-h1">Already retrieved</h1>
            <p class="cb-lede">
                This capsule was set to delete after its first retrieval and has already been
                opened. The content is no longer available.
            </p>
            <p><a href="/" class="cb-btn">Create your own capsule</a></p>
        `;
    }

    #renderBurnWarning() {
        return html`
            <h1 class="cb-h1">This capsule can only be opened once</h1>
            <p class="cb-lede">
                The sender chose to delete this capsule after its first retrieval. Opening it now
                will permanently remove it from the server.
            </p>
            <div class="cb-callout cb-callout-danger">
                <strong>This cannot be undone.</strong>
                After opening, the content will no longer be available — not for you, not for anyone
                else.
            </div>
            <p>
                <button type="button" class="cb-btn" @click=${() => this.#confirmBurn()}>
                    Yes, open and burn
                </button>
                <a href="/" class="cb-btn cb-btn-ghost">Cancel</a>
            </p>
        `;
    }

    #renderPasswordPrompt() {
        return html`
            <h1 class="cb-h1">This capsule is password protected</h1>
            <p class="cb-lede">
                The sender protected this capsule with a password. Enter it below to decrypt the
                content. The password is never sent to the server.
            </p>
            <form class="cb-form" @submit=${(e: SubmitEvent) => this.#handlePasswordSubmit(e)}>
                <label class="cb-label" for="pwd-input">Password</label>
                <input
                    type="password"
                    id="pwd-input"
                    class="cb-input"
                    placeholder="Enter the password"
                    autocomplete="off"
                    required
                    .disabled=${this.passwordBusy}
                />
                <p class="cb-error" ?hidden=${!this.passwordError}>${this.passwordError}</p>
                <div class="cb-kdf-progress" ?hidden=${!this.passwordBusy}>
                    <div class="cb-spinner"></div>
                    <p class="cb-lede cb-pulse">${this.kdfProgressLabel}</p>
                </div>
                <button type="submit" class="cb-btn" .disabled=${this.passwordBusy}>
                    Decrypt capsule
                </button>
            </form>
        `;
    }

    #renderError() {
        return html`
            <h1 class="cb-h1">Not available</h1>
            <p class="cb-lede">
                ${this.errorMessage || 'This capsule does not exist, has expired, or the link is incomplete.'}
            </p>
            <p><a href="/" class="cb-btn">Create your own capsule</a></p>
        `;
    }

    #renderDisplay() {
        const items = this.items;
        const index = Math.min(this.activeIndex, items.length - 1);
        const active = items[index];
        const multi = items.length > 1;

        const totalBytes = items.reduce((sum, item) => sum + (item.bytes?.length ?? 0), 0);
        const statusParts: string[] = [];
        if (multi) statusParts.push(`${items.length} items`);
        statusParts.push(`${formatSize(totalBytes)} total`);
        statusParts.push(formatExpiry(this.expiresAt));

        return html`
            ${
                this.capsuleTitle
                    ? html`<h1 class="cb-h1">${this.capsuleTitle}</h1>`
                    : html`<h1 class="cb-sr-only">Capsule</h1>`
            }
            ${
                active
                    ? html`
                      <div class="cb-viewer-head">
                          ${multi ? this.#renderTabs(items, index) : ''}
                          ${this.#renderItemBar(active, index)}
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
                  `
                    : ''
            }
            <p class="cb-status cb-capsule-status">
                ${statusParts.map((part) => html`<span>${part}</span>`)}
            </p>
        `;
    }

    #renderTabs(items: DisplayItem[], active: number) {
        return html`
            <div class="cb-itemtabs-row">
                <div
                    class="cb-itemtabs"
                    role="tablist"
                    aria-label="Items in this capsule"
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
                <div class="cb-more" hidden>
                    <button
                        type="button"
                        class="cb-btn-secondary cb-more-btn"
                        aria-haspopup="menu"
                        aria-expanded=${this.moreOpen ? 'true' : 'false'}
                        aria-controls="cb-more-menu"
                        @click=${() => {
                            this.moreOpen = !this.moreOpen;
                            if (this.moreOpen) {
                                void this.updateComplete.then(() =>
                                    this.querySelector<HTMLElement>(
                                        '.cb-menu [aria-checked="true"]',
                                    )?.focus(),
                                );
                            }
                        }}
                    >
                        ☰ ${this.overflow.length} more
                    </button>
                    <div
                        id="cb-more-menu"
                        class="cb-menu"
                        role="menu"
                        ?hidden=${!this.moreOpen}
                        @keydown=${(e: KeyboardEvent) => this.#onMenuKey(e)}
                    >
                        ${items.map(
                            (item, i) => html`
                                <button
                                    type="button"
                                    role="menuitemradio"
                                    aria-checked=${i === active ? 'true' : 'false'}
                                    @click=${() => {
                                        this.moreOpen = false;
                                        this.#select(i);
                                    }}
                                >
                                    <span class="cb-itemtab-kind">${kindBadge(item)}</span>
                                    <span class="cb-itemtab-name">${itemName(item, i)}</span>
                                </button>
                            `,
                        )}
                    </div>
                </div>
            </div>
        `;
    }

    /** Hides the tabs that do not fit; the "N more" menu then lists every item. Runs
     *  after every render and on resize; settles after at most one re-render
     *  (the menu's count changes only when the fit does). */
    #fitTabs(): void {
        const row = this.querySelector<HTMLElement>('.cb-itemtabs-row');
        const bar = row?.querySelector<HTMLElement>('[role="tablist"]');
        const more = row?.querySelector<HTMLElement>('.cb-more');
        if (!row || !bar || !more) return;

        if (!this.#resizeObserver) {
            this.#resizeObserver = new ResizeObserver(() => this.#fitTabs());
            this.#resizeObserver.observe(row);
        }

        const hidden = fitTabs(bar, more, this.activeIndex);
        if (hidden.join() !== this.overflow.join()) this.overflow = hidden;
        if (hidden.length === 0 && this.moreOpen) this.moreOpen = false;
    }

    #renderItemBar(item: DisplayItem, index: number) {
        const isMarkdown = item.presentation === 'markdown';
        const isFile = item.presentation === 'download';
        const source = this.sourceView.has(index);
        const copied = this.copiedIndex === index;

        return html`
            <div class="cb-itembar">
                ${
                    this.items.length > 1
                        ? ''
                        : html`<span class="cb-itembar-name" id="cb-item-name"
                          >${itemName(item, index)}</span
                      >`
                }
                <span class="cb-itembar-meta">${itemMeta(item)}</span>
                <span class="cb-itembar-actions">
                    ${
                        isMarkdown
                            ? html`
                              <span class="cb-segmented" role="group" aria-label="Markdown view">
                                  <button
                                      type="button"
                                      aria-pressed=${source ? 'false' : 'true'}
                                      @click=${() => this.#setSource(index, false)}
                                  >
                                      Preview
                                  </button>
                                  <button
                                      type="button"
                                      aria-pressed=${source ? 'true' : 'false'}
                                      @click=${() => this.#setSource(index, true)}
                                  >
                                      Source
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
                                  class="cb-btn-secondary cb-btn-sm"
                                  @click=${() => this.#copy(item, index)}
                              >
                                  ${copied ? 'Copied' : 'Copy'}
                              </button>
                          `
                    }
                    <button
                        type="button"
                        class="cb-btn-secondary cb-btn-sm"
                        @click=${() => downloadItem(item, index)}
                    >
                        Download
                    </button>
                </span>
                <span class="cb-sr-only" aria-live="polite">${copied ? 'Copied to clipboard' : ''}</span>
            </div>
        `;
    }

    #select(index: number): void {
        if (index === this.activeIndex) return;
        this.activeIndex = index;
        this.copiedIndex = null;
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
        const options = [...this.querySelectorAll<HTMLElement>('.cb-menu button')];
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

    async #copy(item: DisplayItem, index: number): Promise<void> {
        try {
            await navigator.clipboard.writeText(item.text);
        } catch {
            // Clipboard access denied (permissions, insecure context): leave the
            // button as it is; selecting the text by hand still works.
            return;
        }
        this.copiedIndex = index;
        clearTimeout(this.#copiedTimer);
        this.#copiedTimer = setTimeout(() => {
            if (this.copiedIndex === index) this.copiedIndex = null;
        }, 2000);
    }

    #confirmBurn(): void {
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
        if (!password || !this.#pwdContext) return;

        const { pwdSalt, iterations } = this.#pwdContext;

        this.passwordBusy = true;
        this.kdfProgressLabel = `Deriving key… ${iterations.toLocaleString()} iterations`;

        try {
            const pwdKey = await derivePasswordKey(password, pwdSalt, iterations);
            this.#resolvePassword?.(pwdKey);
        } catch {
            this.passwordError = 'Something went wrong. Please try again.';
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
            confirmBurn: () => this.#awaitBurnConfirm(),
            promptForPassword: (header, retry) => this.#promptForPassword(header, retry),
        });

        switch (outcome.kind) {
            case 'success':
                this.capsuleTitle = outcome.title;
                this.items = toDisplayItems(outcome.items);
                this.activeIndex = 0;
                this.expiresAt = outcome.expiresAt;
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
    if (retry.reason === 'rejected') {
        return 'Wrong password, or the capsule is no longer available.';
    }
    const wait = retry.retryAfter;
    return wait
        ? `Too many attempts. Try again in ${wait} second${wait === 1 ? '' : 's'}.`
        : 'Too many attempts. Try again in a moment.';
}

/**
 * Removes the key (the fragment) from the address bar and the current history
 * entry, once the link has served its purpose or cannot work any more - so it
 * does not linger on screen, in screenshots or in a shared screen.
 */
function forgetKey(): void {
    history.replaceState(null, '', window.location.pathname);
}

customElements.define('cb-view-page', CbViewPage);

// --- boot --------------------------------------------------------------------

initShell();

// Point the footer's report link at this capsule (`/report?id=…`): only the id
// from the path, never anything from the fragment.
{
    const id = pasteIdFromInput(window.location.pathname);
    const link = document.querySelector<HTMLAnchorElement>('a[href="/report"]');
    if (id && link) link.href = `/report?id=${id}`;
}
