import { batch, createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { AppMenu } from '../app/AppMenu';
import { registerShortcut } from '../app/keyboard';
import { router } from '../app/router';
import { auth } from '../auth/session';
import { reducedMotion, springAnimation } from '../motion/animate';
import { Icon } from '../design/Icon';
import { VirtualList } from '../virtual/VirtualList';
import { Editor } from './Editor';
import { createNoteCommands } from './commands';
import { blobURL, matches, noteDate, text, useNotes, useNoteRecord, type Folder, type Note } from './store';
import './notes.css';
function path() { return router.route().path.split('/').filter(Boolean).map(value => { try { return decodeURIComponent(value); } catch { return ''; } }); }
export default function Notes() { return <Show when={auth.live()} keyed>{_live => <NotesSession/>}</Show>; }
function NotesSession() {
    const commands = createNoteCommands();
    const [dialog, setDialog] = createSignal<'note' | 'folder' | 'delete'>(), [newText, setNewText] = createSignal(''), [newFolderID, setNewFolderID] = createSignal('');
    createEffect(() => {
        if (!dialog()) return;
        const previous = document.activeElement as HTMLElement | null;
        queueMicrotask(() => document.querySelector<HTMLElement>('.notes-modal textarea,.notes-modal input,.notes-modal button')?.focus());
        onCleanup(() => previous?.focus());
    });
    const [deleting, setDeleting] = createSignal<Note>(), [deleted, setDeleted] = createSignal<{ id: string; command: string }>();
    const deletionState = () => auth.live()?.pending().find(command => command.id === deleted()?.command);
    const deletionFailed = () => ['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed', 'indeterminate'].includes(deletionState()?.status ?? '');
    const [folder, setFolder] = createSignal(path()[1] === 'folder' ? path()[2] : '');
    const [sort, setSort] = createSignal('modified'), [query, setQuery] = createSignal(''), [gallery, setGallery] = createSignal(false);
    const [sidebar, setSidebar] = createSignal(innerWidth >= 1024), [collapsed, setCollapsed] = createSignal(new Set<string>());
    const [keyboardSelection, setKeyboardSelection] = createSignal('');
    const selectedID = () => path()[1] !== 'folder' ? path()[1] : undefined;
    const currentFolder = () => path()[1] === 'folder' ? path()[2] : selectedID() ? folder() : '';
    const page = useNotes(() => ({ ...(currentFolder() ? { folder: currentFolder() } : {}), sort: sort(), time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone }));
    const detail = useNoteRecord(selectedID);
    createEffect(() => {
        const record = detail(), entries = page().sections;
        if (!record || !entries || entries.some(group => group.notes.some(entry => entry.note.id === record.id))) return;
        const target = text(record.data.folder_id);
        if (target && target !== currentFolder()) setFolder(target);
    });
    let editorPane!: HTMLElement;
    createEffect(() => {
        selectedID();
        if (!editorPane || reducedMotion()) return;
        const animation = springAnimation(value => { editorPane.style.opacity = String(1 - value * .5); editorPane.style.transform = `translateY(${value * 8}px)`; }, 1, 0, { response: .25, damping: 1 });
        onCleanup(() => { animation.cancel(); editorPane.style.opacity = ''; editorPane.style.transform = ''; });
    });
    const groups = createMemo(() => (page().sections ?? []).map(group => ({ ...group, notes: group.notes.filter(note => matches(note, query()) && !(deleted()?.id === note.note.id && !deletionFailed())) })).filter(group => group.notes.length));
    const rows = createMemo(() => groups().flatMap(group => group.notes.map((entry, index) => ({ entry, heading: index === 0 ? group.title : '' }))));
    const selected = createMemo(() => page().sections?.flatMap(group => group.notes).find(entry => entry.note.id === selectedID()));
    const readOnly = () => Boolean(page().folders?.find(item => item.folder.id === currentFolder())?.read_only) || deleted()?.id === selectedID() && deletionState()?.status === 'indeterminate';
    const writableFolders = () => (page().folders ?? []).filter(item => !item.read_only && !item.recently_deleted);
    const canCreate = () => !readOnly() && writableFolders().length > 0 && commands.capabilities.allowed('notes.create');
    const startNew = (kind: 'note' | 'folder') => { if (kind === 'note' && !canCreate()) return; setNewText(''); setNewFolderID(writableFolders().some(item => item.folder.id === currentFolder()) ? currentFolder() : writableFolders()[0]?.folder.id ?? ''); setDialog(kind); };
    const confirm = (event: Event) => {
        if (dialog() === 'delete') {
            const entry = deleting(); if (!entry) return;
            const command = commands.issue(event, 'notes.delete', { id: entry.note.id }, entry.note.version);
            if (command) { setDeleted({ id: entry.note.id, command }); setDialog(); openFolder(currentFolder()); }
            return;
        }
        const value = dialog() === 'folder' ? newText().trim() : newText(); if (!value.split('\n')[0].trim()) return;
        const lines = value.split('\n'), title = lines.shift()!;
        const command = commands.create(event, dialog() === 'folder' ? 'notes.create_folder' : 'notes.create', dialog() === 'folder' ? { title: value } : { folder_id: newFolderID(), title, plain_text: lines.join('\n') });
        if (command) { const kind = dialog(); setDialog(); if (kind !== 'folder' && newFolderID() && newFolderID() !== currentFolder()) openFolder(newFolderID()); }
    };
    createEffect(() => {
        const pending = commands.creation(), state = commands.creationState();
        if (!pending || !state?.observed_record_ids?.length) return;
        if (pending.op === 'notes.create_folder') {
            const entry = page().folders?.find(item => state.observed_record_ids!.includes(item.folder.id));
            if (entry) { commands.clearCreation(); openFolder(entry.folder.id); }
        } else {
            const entry = page().sections?.flatMap(group => group.notes).find(item => state.observed_record_ids!.includes(item.note.id));
            if (entry) { commands.clearCreation(); openNote(entry); }
        }
    });
    const folderTitle = () => text(page().folders?.find(item => item.folder.id === currentFolder())?.folder.data.title) || 'All Notes';
    const openNote = (entry: Note, keyboard = false) => batch(() => { setKeyboardSelection(keyboard ? entry.note.id : ''); setFolder(currentFolder()); if (innerWidth < 1024) setSidebar(false); router.navigate('/notes/' + encodeURIComponent(entry.note.id)); });
    const openFolder = (id = '') => { setFolder(id); setQuery(''); if (innerWidth < 1024) setSidebar(false); router.navigate(id ? '/notes/folder/' + encodeURIComponent(id) : '/notes'); };
    onMount(() => {
        const disposers = [registerShortcut({ key: 'f', input: true, action: () => document.querySelector<HTMLInputElement>('.notes-search')?.focus() })];
        const escape = (event: KeyboardEvent) => { if (event.key !== 'Escape') return; if (dialog()) { setDialog(); return; } if (query()) setQuery(''); else if (selectedID()) openFolder(currentFolder()); else setSidebar(false); };
        const saveLeaving = (event: Event) => { if (event.target instanceof Element && !event.target.closest('.notes-text-editor') && commands.unsaved()) commands.saveAll(event); };
        const shortcuts = (event: KeyboardEvent) => {
            if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'n') { event.preventDefault(); commands.saveAll(event); startNew('note'); }
            else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') { event.preventDefault(); commands.saveAll(event); }
            else if (event.key === 'Escape' || event.key === 'Tab' && event.target instanceof Element && event.target.closest('.notes-text-editor') || ['ArrowDown', 'ArrowUp'].includes(event.key) && !(event.target instanceof Element && event.target.closest('textarea,input'))) commands.saveAll(event);
        };
        for (const type of ['click', 'pointerup', 'touchend']) document.addEventListener(type, saveLeaving, true);
        document.addEventListener('keydown', shortcuts, true);
        window.addEventListener('keydown', escape);
        onCleanup(() => { for (const type of ['click', 'pointerup', 'touchend']) document.removeEventListener(type, saveLeaving, true); document.removeEventListener('keydown', shortcuts, true); disposers.forEach(dispose => dispose()); window.removeEventListener('keydown', escape); });
    });
    function FolderRow(props: { item: Folder; ancestors: string[] }) {
        const id = () => props.item.folder.id;
        const children = () => (page().folders ?? []).filter(item => item.parent_id === id() && !props.ancestors.includes(item.folder.id));
        const closed = () => collapsed().has(id());
        return <div class="notes-folder-branch"><div class="notes-folder-row"><Show when={children().length}><button class="notes-disclosure" aria-label={'Toggle ' + text(props.item.folder.data.title)} aria-expanded={!closed()} onClick={() => setCollapsed(previous => { const next = new Set(previous); if (next.has(id())) next.delete(id()); else next.add(id()); return next; })}>{closed() ? '›' : '⌄'}</button></Show><button class="notes-folder" classList={{ active: currentFolder() === id() }} onClick={() => openFolder(id())}><Icon name={props.item.recently_deleted ? 'notes' : 'list'}/><span>{text(props.item.folder.data.title) || 'Untitled folder'}</span><small>{props.item.count}</small></button></div><Show when={!closed()}><div class="notes-subfolders"><For each={children()}>{item => <FolderRow item={item} ancestors={[...props.ancestors, id()]}/>}</For></div></Show></div>;
    }
    function Card(props: { entry: Note; heading?: string }) {
        const data = () => props.entry.note.data;
        const thumbnail = () => !data().locked && props.entry.thumbnail ? blobURL(props.entry.thumbnail.digest, props.entry.thumbnail.variant, props.entry.note.id) : undefined;
        return <><Show when={props.heading}><h2 class="notes-group-title">{props.heading}</h2></Show><button class="notes-card" classList={{ selected: selectedID() === props.entry.note.id }} onClick={() => openNote(props.entry)} data-note-id={props.entry.note.id}><Show when={thumbnail()}><img class="notes-thumbnail" src={thumbnail()} alt="" loading="lazy"/></Show><strong>{text(data().title) || 'Untitled note'}<Show when={data().locked}><span aria-label="Locked"> 🔒</span></Show></strong><time>{noteDate(data()[sort() === 'created' ? 'created_at' : 'modified_at'])}</time><Show when={!data().locked}><p>{props.entry.preview || 'No additional text'}</p></Show></button></>;
    }
    return <main class="notes-app" aria-label="Notes" classList={{ 'notes-show-folders': sidebar(), 'notes-show-editor': Boolean(selectedID()) && keyboardSelection() !== selectedID(), 'notes-gallery-mode': gallery() }}>
        <aside class="notes-sidebar"><header><AppMenu/><h1>Notes</h1><button class="icon-button notes-close-folders" aria-label="Close folders" onClick={() => setSidebar(false)}><Icon name="close"/></button></header><button class="notes-folder" classList={{ active: !currentFolder() }} onClick={() => openFolder()}><Icon name="notes"/><span>All Notes</span></button><For each={page().accounts ?? []}>{account => <section><h2>{account.title || 'Notes account'}</h2><For each={(page().folders ?? []).filter(item => item.account_id === account.id && (!item.parent_id || !page().folders?.some(parent => parent.folder.id === item.parent_id)))}>{item => <FolderRow item={item} ancestors={[]}/>}</For></section>}</For><Show when={commands.capabilities.allowed('notes.create_folder')}><button class="notes-folder" onClick={() => startNew('folder')}>+ New Folder</button></Show></aside>
        <section class="notes-list-pane" onKeyDown={event => {
            const entry = selected();
            if (event.key !== 'Enter' || !(event.target instanceof HTMLElement) || event.target.getAttribute('role') !== 'listbox' || !entry) return;
            event.preventDefault();
            openNote(entry);
            if (innerWidth < 700) queueMicrotask(() => editorPane.querySelector<HTMLButtonElement>('.notes-back')?.focus());
        }}><header class="notes-toolbar"><button class="icon-button" aria-label="Show folders" aria-expanded={sidebar()} onClick={() => setSidebar(!sidebar())}><Icon name="list"/></button><h1>{query() ? 'Search results' : folderTitle()}</h1><button class="icon-button" aria-label={gallery() ? 'List view' : 'Gallery view'} aria-pressed={gallery()} onClick={() => setGallery(!gallery())}><Icon name={gallery() ? 'list' : 'grid'}/></button></header><Show when={canCreate()}><button class="notes-new" onClick={() => startNew('note')}>New Note</button></Show><Show when={deletionFailed()}><p class="notes-notice" role="alert">{deletionState()?.status === 'indeterminate' ? 'Delete status unknown. Refresh before trying again.' : 'Couldn’t delete this note. It was restored.'}</p></Show><Show when={commands.error()}><p class="notes-notice" role="alert">{commands.error()}</p></Show><Show when={commands.creation()}><p class="notes-notice" role="status">{commands.creationFailed() ? 'Couldn’t create.' : commands.creationState()?.status === 'indeterminate' ? 'Creation status unknown. Refresh before trying again.' : 'Creating ' + commands.creation()!.title + '…'}<Show when={commands.creationFailed()}><button onClick={commands.clearCreation}>Dismiss</button></Show></p></Show><div class="notes-list-controls"><label class="notes-search-wrap"><Icon name="search"/><input class="notes-search" type="search" aria-label="Search notes" placeholder="Search notes" value={query()} onInput={event => setQuery(event.currentTarget.value)}/></label><select aria-label="Sort notes" value={sort()} onChange={event => setSort(event.currentTarget.value)}><option value="modified">Date edited</option><option value="created">Date created</option><option value="title">Title</option></select></div>
        <Show when={commands.capabilities.state() && commands.capabilities.state() !== 'ready'}><p class="notes-notice" role="status">Notes are unavailable on this Mac. Existing notes may be out of date.</p></Show><Show when={auth.live()?.status() !== 'connected'}><p class="notes-notice" role="status">Reconnecting…</p></Show><Show when={!page().error} fallback={<p role="alert">{page().error === 'capacity' ? 'Too many notes to display. Choose a smaller folder.' : 'Notes unavailable.'} <button onClick={() => location.reload()}>Reload</button></p>}><Show when={page().sections} fallback={<p class="notes-empty" role="status">Loading notes…</p>}><Show when={rows().length} fallback={<p class="notes-empty">{query() ? 'No matching notes' : 'No notes'}</p>}><Show when={gallery()} fallback={<VirtualList items={rows()} keyFor={row => row.entry.note.id} estimate={row => row.heading ? 136 : 108} selectedKey={selectedID()} label="Notes" onActive={row => openNote(row.entry, true)} render={row => <Card entry={row.entry} heading={row.heading}/>} renderReactive={row => <Card entry={row().entry} heading={row().heading}/>}/>}><div class="notes-gallery" aria-label="Notes gallery"><For each={groups()}>{group => <section><h2 class="notes-group-title">{group.title}</h2><div><For each={group.notes}>{entry => <Card entry={entry}/>}</For></div></section>}</For></div></Show></Show></Show></Show><footer class="notes-count">{rows().length} notes</footer></section>
        <section ref={editorPane} class="notes-editor" aria-label="Note content"><header class="notes-toolbar"><button class="notes-back" onClick={() => openFolder(currentFolder())}>‹ Notes</button></header><Show when={selected()} fallback={<p class="notes-empty">{selectedID() ? 'This note is unavailable in this folder.' : 'Select a note'}</p>}>{entry => <Editor entry={entry()} commands={commands} readOnly={readOnly()} remove={() => { setDeleting(entry()); setDialog('delete'); }}/>}</Show></section>
        <Show when={dialog()}><div class="notes-modal-backdrop"><section class="notes-modal" role="dialog" aria-modal="true" aria-label={dialog() === 'delete' ? 'Delete note' : dialog() === 'folder' ? 'New folder' : 'New note'} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setDialog(); } if (event.key === 'Tab') { const fields = [...event.currentTarget.querySelectorAll<HTMLElement>('input,textarea,select,button:not([disabled])')]; const index = fields.indexOf(document.activeElement as HTMLElement); if (event.shiftKey && index <= 0) { event.preventDefault(); fields.at(-1)?.focus(); } else if (!event.shiftKey && index === fields.length - 1) { event.preventDefault(); fields[0]?.focus(); } } }}>
            <h2>{dialog() === 'delete' ? 'Delete this note?' : dialog() === 'folder' ? 'New Folder' : 'New Note'}</h2>
            <Show when={dialog() !== 'delete'} fallback={<p>Delete “{text(deleting()?.note.data.title)}”? This action removes the note from Notes.</p>}><Show when={dialog() === 'folder'} fallback={<><label>Folder<select aria-label="New note folder" value={newFolderID()} onChange={event => setNewFolderID(event.currentTarget.value)}><For each={writableFolders()}>{item => <option value={item.folder.id}>{text(item.folder.data.title)}</option>}</For></select></label><textarea aria-label="New note text" placeholder="First line is the title" value={newText()} onInput={event => setNewText(event.currentTarget.value)} ref={element => queueMicrotask(() => element.focus())}/></>}><input aria-label="Folder name" value={newText()} onInput={event => setNewText(event.currentTarget.value)} ref={element => queueMicrotask(() => element.focus())}/></Show></Show>
            <div><button onClick={() => setDialog()}>Cancel</button><button disabled={dialog() !== 'delete' && !newText().split('\n')[0].trim()} onClick={confirm}>{dialog() === 'delete' ? 'Delete' : 'Create'}</button></div>
        </section></div></Show>
    </main>;
}
