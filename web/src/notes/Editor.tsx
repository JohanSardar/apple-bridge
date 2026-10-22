import { createEffect, onCleanup, Show, untrack } from 'solid-js';
import { externalBody, useNoteBody } from './body';
import { RichNote } from './RichNote';
import type { createNoteCommands } from './commands';
import { noteDate, object, text, type Note } from './store';
export function Editor(props: { entry: Note; commands: ReturnType<typeof createNoteCommands>; readOnly: boolean; remove: () => void }) {
    const record = () => props.entry.note;
    const body = useNoteBody(record);
    const editable = () => !props.readOnly && !props.entry.read_only && !record().data.locked && !externalBody(record().data) && record().data.editable === 'plain' && props.commands.capabilities.allowed('notes.update');
    createEffect(() => { if (!editable()) return; const current = record(); untrack(() => props.commands.begin(current)); onCleanup(() => props.commands.release(current.id)); });
    const draft = () => props.commands.drafts().get(record().id);
    return <article><time class="notes-modified">{noteDate(record().data.modified_at)}</time><Show when={record().data.locked || editable() || !Array.isArray(body.data().blocks) || !(body.data().blocks as unknown[]).some(block => object(block).type === 'title')}><h1>{text(record().data.title) || 'Untitled note'}</h1></Show><Show when={!record().data.locked} fallback={<p class="notes-notice">This note is locked. Unlock it in Notes on your Mac.</p>}>
        <Show when={editable()} fallback={<><p class="notes-notice">{props.readOnly || props.entry.read_only ? 'This folder is read-only.' : externalBody(record().data) || record().data.editable !== 'plain' ? 'This content is read-only. Edit it in Notes on your Mac.' : 'Editing is unavailable on this Mac.'}</p><Show when={body.status() !== 'loading'} fallback={<p class="notes-notice" role="status">Loading note content…</p>}><Show when={body.status() !== 'unavailable'} fallback={<p class="notes-notice" role="alert">Note content unavailable. <button onClick={body.retry}>Retry</button></p>}><RichNote data={body.data()}/></Show></Show></>}>
            <div class="notes-save-bar"><span role="status">{draft()?.command ? 'Saving…' : draft() && props.commands.dirty(draft()!) ? 'Edited' : 'Saved'}</span><button onClick={event => props.commands.save(event, record().id)}>Done</button></div>
            <Show when={draft()?.error}><div class="notes-notice" role="alert">{draft()?.error}<Show when={draft()?.conflict}><p>Your draft is kept. Reload to discard it, or keep editing and copy your text before reloading.</p><button disabled={!props.commands.canReload(record().id)} onClick={() => props.commands.reload(record().id)}>Reload note</button></Show></div></Show>
            <textarea class="notes-text-editor" aria-label="Note text" value={draft()?.text ?? text(record().data.plain_text)} onInput={event => props.commands.edit(record().id, event.currentTarget.value)} onKeyUp={event => { if (!event.isComposing) props.commands.save(event, record().id, true); }} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') { event.preventDefault(); props.commands.save(event, record().id); } }}/>
        </Show>
    </Show><Show when={!record().data.locked && !props.readOnly && !props.entry.read_only && props.commands.capabilities.allowed('notes.delete')}><button class="notes-delete" onClick={props.remove}>Delete note</button></Show></article>;
}
