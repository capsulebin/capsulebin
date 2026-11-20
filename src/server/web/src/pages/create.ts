/**
 * Create page: editor, encryption and share.
 *
 * The flow: plaintext → capsule bytes → AES-256-GCM → opaque POST.
 * The server sees ciphertext only.
 */

import { MAX_ITEM_BYTES, MAX_ITEMS, Paste, PasteApi, generateUlid } from '@capsulebin/core';
import {
    byAttr,
    byId,
    initShell,
    languageFromNextPage,
    setValue,
    showTemplate,
    setText,
} from './shell';
import { apiErrorText } from './errors';
import { fitTabs, nextTabIndex, revealActiveItem } from './item-tabs';
import { formatSize, t, type TextKey } from '../language';
import { renderMarkdown } from '../render-markdown';
import { trustedHtml } from '../trusted-html';

/** Hard file-size rejection before FileReader — matches server default. */
const MAX_FILE_BYTES = 10 * 1024 * 1024;

/** Item names are display and download metadata (≤ 255 UTF-8 bytes in the
 *  capsule format); kept well below that so tabs stay readable. */
const MAX_NAME_CHARS = 200;

// --- item state ----------------------------------------------------------

type Presentation = 'plain' | 'code' | 'markdown' | 'download';

interface ItemState {
    id: string;
    presentation: Presentation;
    text: string;
    codeLang: string;
    /** What the recipient sees and saves the item as. */
    name: string;
    /** Once the user edits the name, type and language changes stop renaming it. */
    nameEdited: boolean;
    fileBytes: Uint8Array | null;
    previewActive: boolean;
}

let items: ItemState[] = [];
let activeId: string | null = null;
let addMenuOpen = false;
let moreOpen = false;
/** Items whose tabs do not fit in the bar (counted on the "N more" button). */
let overflow: number[] = [];

/** Listeners on `document` and the resize observer belong to one edit form;
 *  ended when the form is replaced (encrypting, done, error), so a retry does
 *  not register them twice and nothing fires against a removed form. */
let editSession: AbortController | undefined;
let tabsObserver: ResizeObserver | undefined;

function endEditSession(): void {
    editSession?.abort();
    editSession = undefined;
    tabsObserver?.disconnect();
    tabsObserver = undefined;
}

const KIND_BADGE: Record<Presentation, TextKey> = {
    plain: 'item.badge.plain',
    code: 'item.badge.code',
    markdown: 'item.badge.markdown',
    download: 'item.badge.download',
};

const LANG_EXTENSIONS: Record<string, string> = {
    bash: 'sh',
    css: 'css',
    diff: 'diff',
    html: 'html',
    ini: 'toml',
    javascript: 'js',
    json: 'json',
    markdown: 'md',
    python: 'py',
    rust: 'rs',
    sql: 'sql',
    typescript: 'ts',
    yaml: 'yaml',
};

/** A suggested name for a text item, unique within the capsule. */
function defaultName(item: ItemState): string {
    const base =
        item.presentation === 'markdown'
            ? 'notes.md'
            : item.presentation === 'code'
              ? `snippet.${LANG_EXTENSIONS[item.codeLang] ?? 'txt'}`
              : 'text.txt';
    const taken = new Set(items.filter((other) => other !== item).map((other) => other.name));
    if (!taken.has(base)) return base;
    const dot = base.lastIndexOf('.');
    for (let n = 2; ; n++) {
        const candidate = `${base.slice(0, dot)}-${n}${base.slice(dot)}`;
        if (!taken.has(candidate)) return candidate;
    }
}

/** No path separators or control characters; trimmed and bounded. */
function cleanName(name: string): string {
    // eslint-disable-next-line no-control-regex
    return name
        .replace(/[\u0000-\u001f\u007f\\/]/g, '_')
        .trim()
        .slice(0, MAX_NAME_CHARS);
}

function newItem(presentation: Presentation): ItemState {
    const item: ItemState = {
        id: generateUlid(),
        presentation,
        text: '',
        codeLang: 'plaintext',
        name: '',
        nameEdited: false,
        fileBytes: null,
        previewActive: false,
    };
    if (presentation !== 'download') item.name = defaultName(item);
    return item;
}

function getActive(): ItemState | undefined {
    return items.find((item) => item.id === activeId);
}

function isItemEmpty(item: ItemState): boolean {
    return item.presentation === 'download' ? !item.fileBytes : item.text.trim().length === 0;
}

/** The item editor's controls show the active item and are set by the script;
 *  what the user did to the items is in `items`. */
const ITEM_CONTROLS =
    '[data-item-type], [data-item-name], [data-item-lang], [data-item-text], [data-item-file]';

/** Whether a form control no longer holds the value the template gave it. */
function isChanged(control: Element): boolean {
    if (control instanceof HTMLInputElement) {
        return control.type === 'checkbox' || control.type === 'radio'
            ? control.checked !== control.defaultChecked
            : control.value !== control.defaultValue;
    }
    if (control instanceof HTMLSelectElement) {
        const options = [...control.options];
        const initial = options.find((option) => option.defaultSelected) ?? options[0];
        return control.value !== (initial?.value ?? '');
    }
    return control instanceof HTMLTextAreaElement && control.value !== control.defaultValue;
}

/** A draft is worth keeping once an item has content or a name the user gave it,
 *  or any other field of the form (title, expiry, burn, password) was changed. */
function hasDraft(): boolean {
    if (items.some((item) => !isItemEmpty(item) || item.nameEdited)) return true;
    const form = document.querySelector<HTMLFormElement>('[data-create-form]');
    return [...(form?.elements ?? [])].some(
        (control) => !control.matches(ITEM_CONTROLS) && isChanged(control),
    );
}

/** Reloading the page under another prefix would discard the draft: from the
 *  first input on, the language switch only stores the choice. Stays so even
 *  if the form is emptied again (and on the done page, which holds the links). */
function keepDraftOnLanguageSwitch(): void {
    if (hasDraft()) languageFromNextPage(shell);
}

function itemByteLength(item: ItemState): number {
    return item.presentation === 'download'
        ? (item.fileBytes?.length ?? 0)
        : new TextEncoder().encode(item.text).length;
}

function totalPayloadBytes(): number {
    return items.reduce((sum, item) => sum + itemByteLength(item), 0);
}

function displayName(item: ItemState, index: number): string {
    return (
        item.name ||
        (item.presentation === 'download'
            ? t('item.kind.download')
            : `${t('item.kind.other')} ${index + 1}`)
    );
}

// --- phases ------------------------------------------------------------------

function showEdit(): void {
    endEditSession();
    editSession = new AbortController();
    const { signal } = editSession;
    showTemplate('tmpl-edit', 'data-create-root');

    items = [newItem('plain')];
    activeId = items[0]!.id;
    addMenuOpen = false;
    moreOpen = false;
    overflow = [];

    const form = byAttr<HTMLFormElement>('create-form');
    // Bubbles up after the field handlers below have updated the items; added
    // files and type changes go through render(). Checkboxes and selects fire
    // `input` too, but `change` is what every browser sends for them.
    form.addEventListener('input', keepDraftOnLanguageSwitch);
    form.addEventListener('change', keepDraftOnLanguageSwitch);

    // Tabs: selection, removal and keyboard navigation (delegated, since the
    // tab list is re-rendered).
    const tabs = byAttr<HTMLElement>('item-tabs');
    tabs.addEventListener('click', (event) => {
        const target = event.target as HTMLElement;
        const remove = target.closest<HTMLElement>('[data-remove-id]');
        if (remove) {
            removeItem(remove.dataset.removeId!);
            return;
        }
        const tab = target.closest<HTMLElement>('[data-tab-id]');
        if (tab) selectItem(tab.dataset.tabId!);
    });
    tabs.addEventListener('keydown', (event) => {
        // From the focused tab; with the roving tabindex that is normally the
        // selected one.
        const focused = (event.target as HTMLElement).closest<HTMLElement>('[data-tab-id]');
        const index = items.findIndex((item) => item.id === (focused?.dataset.tabId ?? activeId));
        const next = nextTabIndex(event.key, index, items.length);
        if (next === null) return;
        event.preventDefault();
        selectItem(items[next]!.id);
    });

    // "+ Add item" menu.
    const addButton = byAttr<HTMLButtonElement>('item-add');
    const addMenu = byAttr<HTMLElement>('add-menu');
    addButton.addEventListener('click', () => setAddMenu(!addMenuOpen));
    for (const option of addMenu.querySelectorAll<HTMLButtonElement>('[data-add-type]')) {
        option.addEventListener('click', () => {
            setAddMenu(false);
            const type = option.dataset.addType as Presentation;
            addItem(type);
            if (type === 'download') byAttr<HTMLInputElement>('item-file').click();
        });
    }
    addMenu.addEventListener('keydown', (event) => {
        const options = [...addMenu.querySelectorAll<HTMLButtonElement>('[data-add-type]')];
        const index = options.indexOf(document.activeElement as HTMLButtonElement);
        const key =
            event.key === 'ArrowDown'
                ? 'ArrowRight'
                : event.key === 'ArrowUp'
                  ? 'ArrowLeft'
                  : event.key;
        const next = nextTabIndex(key, index, options.length);
        if (next !== null) {
            event.preventDefault();
            options[next]!.focus();
        }
    });
    document.addEventListener(
        'click',
        (event) => {
            if (!addMenuOpen) return;
            const target = event.target as Node;
            if (!addMenu.contains(target) && !addButton.contains(target)) setAddMenu(false);
        },
        { signal },
    );
    document.addEventListener(
        'keydown',
        (event) => {
            if (event.key === 'Escape' && addMenuOpen) {
                setAddMenu(false);
                addButton.focus();
            }
        },
        { signal },
    );

    // "N more" menu for tabs that do not fit, refitted whenever the row resizes.
    const moreButton = byAttr<HTMLButtonElement>('more-btn');
    const moreMenu = byAttr<HTMLElement>('more-menu');
    moreButton.addEventListener('click', () => setMoreMenu(!moreOpen));
    moreMenu.addEventListener('keydown', (event) => {
        const options = [...moreMenu.querySelectorAll<HTMLButtonElement>('button')];
        const index = options.indexOf(document.activeElement as HTMLButtonElement);
        const key =
            event.key === 'ArrowDown'
                ? 'ArrowRight'
                : event.key === 'ArrowUp'
                  ? 'ArrowLeft'
                  : event.key;
        const next = nextTabIndex(key, index, options.length);
        if (next !== null) {
            event.preventDefault();
            options[next]!.focus();
        }
    });
    moreMenu.addEventListener('click', (event) => {
        const option = (event.target as HTMLElement).closest<HTMLElement>('[data-more-id]');
        if (!option) return;
        setMoreMenu(false);
        selectItem(option.dataset.moreId!);
    });
    const more = byAttr<HTMLElement>('more');
    document.addEventListener(
        'click',
        (event) => {
            if (moreOpen && !more.contains(event.target as Node)) setMoreMenu(false);
        },
        { signal },
    );
    document.addEventListener(
        'keydown',
        (event) => {
            if (event.key === 'Escape' && moreOpen) {
                setMoreMenu(false);
                moreButton.focus();
            }
        },
        { signal },
    );
    tabsObserver = new ResizeObserver(() => layoutTabs());
    tabsObserver.observe(byAttr<HTMLElement>('tabs-row'));

    // Item bar.
    const typeSelect = byAttr<HTMLSelectElement>('item-type');
    typeSelect.addEventListener('change', () => {
        const active = getActive();
        if (active) changeType(active, typeSelect.value as Presentation);
    });

    const nameInput = byAttr<HTMLInputElement>('item-name');
    nameInput.addEventListener('input', () => {
        const active = getActive();
        if (!active) return;
        active.name = cleanName(nameInput.value);
        active.nameEdited = active.name.length > 0;
        renderTabs();
    });
    nameInput.addEventListener('blur', () => {
        const active = getActive();
        if (active && !active.name && active.presentation !== 'download') {
            active.name = defaultName(active);
            active.nameEdited = false;
            render();
        }
    });

    const langSelect = byAttr<HTMLSelectElement>('item-lang');
    langSelect.addEventListener('change', () => {
        const active = getActive();
        if (!active) return;
        active.codeLang = langSelect.value;
        if (!active.nameEdited) active.name = defaultName(active);
        render();
    });

    byAttr<HTMLButtonElement>('preview-write').addEventListener('click', () => setPreview(false));
    byAttr<HTMLButtonElement>('preview-show').addEventListener('click', () => setPreview(true));
    byAttr<HTMLButtonElement>('item-remove').addEventListener('click', () => {
        if (activeId) removeItem(activeId);
    });

    // Content.
    const textArea = byAttr<HTMLTextAreaElement>('item-text');
    textArea.addEventListener('input', () => {
        const active = getActive();
        if (!active) return;
        active.text = textArea.value;
        refreshSizeMeter();
        renderTypeOptions(active);
    });

    const fileInput = byAttr<HTMLInputElement>('item-file');
    fileInput.addEventListener('change', () => {
        if (fileInput.files) addFiles(fileInput.files);
        fileInput.value = '';
    });
    byAttr<HTMLButtonElement>('item-choose').addEventListener('click', () => fileInput.click());

    // Files can be dropped anywhere on the form, not only on a file item.
    form.addEventListener('dragover', (event) => {
        if (!event.dataTransfer?.types.includes('Files')) return;
        event.preventDefault();
        form.classList.add('cb-dragover');
    });
    form.addEventListener('dragleave', (event) => {
        if (!form.contains(event.relatedTarget as Node | null))
            form.classList.remove('cb-dragover');
    });
    form.addEventListener('drop', (event) => {
        if (!event.dataTransfer?.files.length) return;
        event.preventDefault();
        form.classList.remove('cb-dragover');
        addFiles(event.dataTransfer.files);
    });

    form.addEventListener('submit', async (event) => {
        event.preventDefault();

        const itemData = collectItems();
        if (itemData.length === 0) {
            byAttr<HTMLTextAreaElement>('item-text').focus();
            return;
        }
        if (totalPayloadBytes() > MAX_ITEM_BYTES) return;

        const expiryValue = byId<HTMLSelectElement>('create-expiry').value;
        const expirySecs = expiryValue === '' ? null : parseInt(expiryValue, 10);
        if (expirySecs !== null && (isNaN(expirySecs) || expirySecs <= 0)) return;
        const burn = byId<HTMLInputElement>('create-burn').checked;
        const passwordField = byId<HTMLInputElement>('create-password');
        const password = passwordField.value;
        // A password of only whitespace is refused here, not passed on: the core
        // would drop it and create the capsule without protection although the
        // creator typed one. An empty field means no password; spaces around
        // real characters stay part of the password.
        const passwordError = byAttr('password-error');
        const blankPassword = password !== '' && password.trim() === '';
        passwordError.textContent = blankPassword ? t('create.passwordBlank') : '';
        passwordError.hidden = !blankPassword;
        if (blankPassword) {
            passwordField.setAttribute('aria-invalid', 'true');
            passwordField.focus();
            return;
        }
        passwordField.removeAttribute('aria-invalid');
        // Encrypted with the items; the server never sees it.
        const title = byId<HTMLInputElement>('create-title').value.trim() || null;

        showEncrypting();

        try {
            const { shareUrl, deleteUrl } = await encryptAndShare(
                itemData,
                expirySecs,
                burn,
                password || null,
                title,
            );
            showDone(shareUrl, deleteUrl);
        } catch (err) {
            console.error(err);
            showError(apiErrorText(err));
        }
    });

    render();
}

function selectItem(id: string): void {
    if (id === activeId) return;
    // The tabs are rebuilt on render; keep keyboard focus on the tab bar if it
    // was there, or arrow keys stop working after a click.
    const focusInTabs = byAttr<HTMLElement>('item-tabs').contains(document.activeElement);
    activeId = id;
    render();
    if (focusInTabs) {
        byAttr<HTMLElement>('item-tabs')
            .querySelector<HTMLElement>(`[data-tab-id="${id}"]`)
            ?.focus();
    }
    revealActiveItem(
        document.querySelector<HTMLElement>('.cb-viewer-head'),
        byAttr<HTMLElement>('item-panel'),
    );
}

function setAddMenu(open: boolean): void {
    addMenuOpen = open && items.length < MAX_ITEMS;
    const menu = byAttr<HTMLElement>('add-menu');
    menu.hidden = !addMenuOpen;
    byAttr<HTMLButtonElement>('item-add').setAttribute('aria-expanded', String(addMenuOpen));
    if (addMenuOpen) menu.querySelector<HTMLButtonElement>('[data-add-type]')?.focus();
}

/** Adds an item after the last one and selects it. An empty first item is
 *  replaced instead, so "+ Add item → Code" on a fresh page does not leave an
 *  empty text item behind. */
function addItem(presentation: Presentation): ItemState | undefined {
    const active = getActive();
    if (items.length === 1 && active && isItemEmpty(active) && !active.nameEdited) {
        changeType(active, presentation);
        return active;
    }
    if (items.length >= MAX_ITEMS) return undefined;
    const created = newItem(presentation);
    items.push(created);
    activeId = created.id;
    render();
    if (presentation !== 'download') byAttr<HTMLTextAreaElement>('item-text').focus();
    return created;
}

/** Text, code and Markdown convert into each other keeping the content. A file
 *  and text cannot: switching is only offered while the item is empty. */
function changeType(item: ItemState, presentation: Presentation): void {
    if (item.presentation === presentation) return;
    const toOrFromFile = item.presentation === 'download' || presentation === 'download';
    if (toOrFromFile && !isItemEmpty(item)) {
        render();
        return;
    }
    item.presentation = presentation;
    item.previewActive = false;
    if (presentation === 'download') {
        item.text = '';
        item.name = '';
        item.nameEdited = false;
    } else if (!item.nameEdited) {
        item.fileBytes = null;
        item.name = defaultName(item);
    }
    render();
}

function addFiles(files: FileList): void {
    for (const file of Array.from(files)) {
        if (file.size > MAX_FILE_BYTES) {
            alert(t('create.fileTooLarge', { name: file.name, limit: formatSize(MAX_FILE_BYTES) }));
            continue;
        }
        const active = getActive();
        // Fill an empty item in place (a fresh page, or a just-added File item);
        // otherwise every file becomes an item of its own.
        const target =
            active &&
            isItemEmpty(active) &&
            (active.presentation === 'download' || !active.nameEdited)
                ? active
                : addItem('download');
        if (!target) {
            alert(t('create.tooManyItems', { max: MAX_ITEMS }));
            return;
        }
        target.presentation = 'download';
        target.text = '';
        target.name = cleanName(file.name) || 'file';
        target.nameEdited = true;
        const reader = new FileReader();
        reader.onload = () => {
            target.fileBytes = new Uint8Array(reader.result as ArrayBuffer);
            render();
        };
        reader.readAsArrayBuffer(file);
    }
    render();
}

function removeItem(id: string): void {
    const index = items.findIndex((item) => item.id === id);
    if (index < 0 || items.length <= 1) return;
    items.splice(index, 1);
    if (activeId === id) activeId = items[Math.min(index, items.length - 1)]!.id;
    render();
}

function setPreview(on: boolean): void {
    const active = getActive();
    if (!active || active.presentation !== 'markdown') return;
    active.previewActive = on;
    render();
}

function updatePreview(item: ItemState): void {
    const previewArea = byAttr<HTMLElement>('item-preview');
    const html = renderMarkdown(item.text);
    previewArea.innerHTML = trustedHtml(`<div class="cb-prose">${html}</div>`);
}

/** Keeps the size line and the submit button's disabled state live while
 *  typing, without a full render() (which would reset the textarea's value
 *  and cursor position). Disabling the button here means an oversized capsule
 *  never gets far enough to hit the server's 413. */
function refreshSizeMeter(): void {
    const totalBytes = totalPayloadBytes();
    const overBudget = totalBytes > MAX_ITEM_BYTES;

    const size = { size: formatSize(totalBytes), max: formatSize(MAX_ITEM_BYTES) };
    const parts = [];
    if (items.length > 1)
        parts.push(t('create.itemCount', { count: items.length, max: MAX_ITEMS }));
    parts.push(
        overBudget
            ? t('create.sizeOver', { ...size, over: formatSize(totalBytes - MAX_ITEM_BYTES) })
            : t('create.size', size),
    );

    const meter = byAttr<HTMLElement>('size-meter');
    meter.classList.toggle('cb-size-meter-over', overBudget);
    meter.textContent = parts.join(' · ');

    byAttr<HTMLButtonElement>('create-submit').disabled = overBudget;
}

function render(): void {
    keepDraftOnLanguageSwitch();
    if (!getActive()) activeId = items[0]?.id ?? null;
    renderTabs();
    renderEditor(getActive());
    refreshSizeMeter();
    byAttr<HTMLButtonElement>('item-add').disabled = items.length >= MAX_ITEMS;
}

function setMoreMenu(open: boolean): void {
    moreOpen = open && overflow.length > 0;
    byAttr<HTMLElement>('more-menu').hidden = !moreOpen;
    byAttr<HTMLButtonElement>('more-btn').setAttribute('aria-expanded', String(moreOpen));
    if (moreOpen)
        byAttr<HTMLElement>('more-menu')
            .querySelector<HTMLElement>('[aria-checked="true"]')
            ?.focus();
}

/** Hides the tabs that do not fit and fills the "N more" menu. */
function layoutTabs(): void {
    const index = items.findIndex((item) => item.id === activeId);
    overflow = fitTabs(
        byAttr<HTMLElement>('item-tabs'),
        byAttr<HTMLElement>('more'),
        Math.max(0, index),
    );
    byAttr<HTMLElement>('more-btn').textContent = t('items.more', { count: overflow.length });
    // Once anything overflows, the menu lists every item - one complete list is
    // easier to navigate than "whatever did not fit".
    byAttr<HTMLElement>('more-menu').replaceChildren(
        ...items.map((item, i) => {
            const option = document.createElement('button');
            option.type = 'button';
            option.setAttribute('role', 'menuitemradio');
            option.setAttribute('aria-checked', String(item.id === activeId));
            option.dataset.moreId = item.id;
            const kind = document.createElement('span');
            kind.className = 'cb-itemtab-kind';
            kind.textContent = t(KIND_BADGE[item.presentation]);
            const name = document.createElement('span');
            name.className = 'cb-itemtab-name';
            name.textContent = displayName(item, i);
            option.append(kind, name);
            return option;
        }),
    );
    if (overflow.length === 0 && moreOpen) setMoreMenu(false);
}

function renderTabs(): void {
    const tabs = byAttr<HTMLElement>('item-tabs');
    const removable = items.length > 1;
    tabs.replaceChildren(
        ...items.map((item, index) => {
            const wrap = document.createElement('span');
            wrap.className = 'cb-itemtab-wrap';
            wrap.setAttribute('role', 'presentation');

            const tab = document.createElement('button');
            tab.type = 'button';
            tab.className = 'cb-itemtab';
            tab.setAttribute('role', 'tab');
            tab.id = `cb-create-tab-${item.id}`;
            tab.dataset.tabId = item.id;
            const selected = item.id === activeId;
            tab.setAttribute('aria-selected', String(selected));
            tab.setAttribute('aria-controls', 'cb-create-panel');
            tab.tabIndex = selected ? 0 : -1;

            const kind = document.createElement('span');
            kind.className = 'cb-itemtab-kind';
            kind.textContent = t(KIND_BADGE[item.presentation]);
            const name = document.createElement('span');
            name.className = 'cb-itemtab-name';
            name.textContent = displayName(item, index);
            tab.append(kind, name);
            wrap.append(tab);

            if (removable) {
                const remove = document.createElement('button');
                remove.type = 'button';
                remove.className = 'cb-itemtab-close';
                remove.dataset.removeId = item.id;
                remove.tabIndex = -1;
                remove.textContent = '×';
                remove.title = t('create.removeItem', { name: displayName(item, index) });
                remove.setAttribute('aria-label', remove.title);
                wrap.append(remove);
            }
            return wrap;
        }),
    );
    byAttr<HTMLElement>('item-panel').setAttribute(
        'aria-labelledby',
        activeId ? `cb-create-tab-${activeId}` : '',
    );
    layoutTabs();
}

/** The type select only offers what keeps the content: no switch between file
 *  and text once either has content. */
function renderTypeOptions(item: ItemState): void {
    const typeSelect = byAttr<HTMLSelectElement>('item-type');
    const empty = isItemEmpty(item);
    for (const option of typeSelect.options) {
        const crossesFile = (option.value === 'download') !== (item.presentation === 'download');
        option.disabled = crossesFile && !empty;
    }
}

function renderEditor(active: ItemState | undefined): void {
    const typeSelect = byAttr<HTMLSelectElement>('item-type');
    const nameInput = byAttr<HTMLInputElement>('item-name');
    const langSelect = byAttr<HTMLSelectElement>('item-lang');
    const previewSwitch = byAttr<HTMLElement>('preview-switch');
    const removeButton = byAttr<HTMLButtonElement>('item-remove');
    const textArea = byAttr<HTMLTextAreaElement>('item-text');
    const previewArea = byAttr<HTMLElement>('item-preview');
    const drop = byAttr<HTMLElement>('item-drop');
    const dropLabel = byAttr<HTMLElement>('item-drop-label');
    if (!active) return;

    const isFile = active.presentation === 'download';
    const isCode = active.presentation === 'code';
    const isMarkdown = active.presentation === 'markdown';

    typeSelect.value = active.presentation;
    renderTypeOptions(active);
    if (document.activeElement !== nameInput) nameInput.value = active.name;
    nameInput.placeholder = t(isFile ? 'create.fileNamePlaceholder' : 'create.itemNamePlaceholder');
    langSelect.hidden = !isCode;
    if (isCode) langSelect.value = active.codeLang;
    previewSwitch.hidden = !isMarkdown;
    byAttr('preview-write').setAttribute('aria-pressed', String(!active.previewActive));
    byAttr('preview-show').setAttribute('aria-pressed', String(active.previewActive));
    removeButton.hidden = items.length <= 1;

    drop.hidden = !isFile;
    dropLabel.textContent = active.fileBytes
        ? `${active.name} (${formatSize(itemByteLength(active))})`
        : t('create.dropAnywhere');

    const showText = !isFile && !(isMarkdown && active.previewActive);
    textArea.hidden = !showText;
    if (!isFile) textArea.value = active.text;
    textArea.placeholder = t(
        isCode
            ? 'create.codePlaceholder'
            : isMarkdown
              ? 'create.markdownPlaceholder'
              : 'create.textPlaceholder',
    );

    previewArea.hidden = !(isMarkdown && active.previewActive);
    if (isMarkdown && active.previewActive) updatePreview(active);
}

interface ItemData {
    text: string;
    presentation: Presentation;
    codeLang: string;
    name: string;
    fileBytes: Uint8Array | null;
}

function collectItems(): ItemData[] {
    const result: ItemData[] = [];
    for (const [index, item] of items.entries()) {
        if (isItemEmpty(item)) continue;
        result.push({
            text: item.text,
            presentation: item.presentation,
            codeLang: item.codeLang,
            name: item.name || displayName(item, index),
            fileBytes: item.fileBytes,
        });
    }
    return result;
}

function showEncrypting(): void {
    endEditSession();
    showTemplate('tmpl-encrypting', 'data-create-root');
}

function showDone(shareUrl: string, deleteUrl: string): void {
    showTemplate('tmpl-done', 'data-create-root');

    setValue('share-link', shareUrl);
    setValue('delete-link', deleteUrl);

    for (const btn of document.querySelectorAll<HTMLButtonElement>('[data-copy]')) {
        btn.addEventListener('click', () => {
            const inputId = btn.dataset.copy!;
            const input = byId<HTMLInputElement>(inputId);
            input.select();
            navigator.clipboard.writeText(input.value).catch(() => {});
        });
    }
}

function showError(message: string): void {
    showTemplate('tmpl-error', 'data-create-root');
    setText('error-message', message);
    byAttr('retry-btn').addEventListener('click', showEdit);
}

// --- crypto flow -------------------------------------------------------------

async function encryptAndShare(
    itemData: ItemData[],
    expirySecs: number | null,
    burn: boolean,
    password: string | null,
    title: string | null,
): Promise<{ shareUrl: string; deleteUrl: string }> {
    const paste = new Paste({ title, password, expiresIn: expirySecs, burnAfterRetrieval: burn });

    // One capsule item per input, in the order the creator arranged them.
    for (const item of itemData) {
        if (item.presentation === 'download' && item.fileBytes) {
            paste.addItem({
                id: generateUlid(),
                name: item.name,
                size: item.fileBytes.length,
                mediaType: 'application/octet-stream',
                presentation: 'download',
                bytes: item.fileBytes,
            });
            continue;
        }

        const bytes = new TextEncoder().encode(item.text);
        paste.addItem({
            id: generateUlid(),
            name: item.name,
            size: bytes.length,
            mediaType: item.presentation === 'markdown' ? 'text/markdown' : 'text/plain',
            presentation: item.presentation,
            ...(item.presentation === 'code' ? { language: item.codeLang } : {}),
            bytes,
        });
    }

    return paste.publish(new PasteApi(''), window.location.origin);
}

// --- boot --------------------------------------------------------------------

const shell = initShell();
showEdit();
