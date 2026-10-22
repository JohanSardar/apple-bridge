import { batch, createEffect, createMemo, createSignal, For, onCleanup, untrack } from 'solid-js';
import { auth } from '../../auth/session';
import { createMessageCommands, messageCommandLabel, type MessageFile } from './commands';
export function CommandsFixture() {
    const live = auth.live()!;
    const chatID = new URLSearchParams(location.search).get('chat') ?? '';
    const commands = createMessageCommands(live), thread = live.subscribe('thread', { chat_id: chatID }, { cache: false });
    const [text, setText] = createSignal(''), [id, setID] = createSignal(''), [error, setError] = createSignal('');
    const [files, setFiles] = createSignal<MessageFile[]>([]), [uploading, setUploading] = createSignal(false);
    const [originalFiles,setOriginalFiles]=createSignal<File[]>([]), [retryFile,setRetryFile]=createSignal<MessageFile>();
    const stage = async (selected: File[]) => {
        setUploading(true); setError('');
        try {
            const ready: MessageFile[] = [];
            for (const file of selected) {
                const response = await fetch('/web/v1/upload', { method: 'POST', body: file });
                const value = await response.json();
                if (!response.ok || value.state !== 'ready') throw new Error('Fixture upload was not ready');
                ready.push({ upload_id: value.upload_id, name: file.name });
            }
            setFiles(ready);
        } catch { setError('Fixture upload failed'); }
        finally { setUploading(false); }
    };
    const [captured, setCaptured] = createSignal({ items: thread.data().items ?? [], cursor: thread.cursor() });
    createEffect(() => { const items = thread.data().items ?? [], cursor = thread.cursor(); untrack(() => batch(() => { commands.observe(items, cursor, chatID); setCaptured({ items, cursor }); })); });
    const rows = createMemo(() => [...commands.projectCaptured(captured().items, captured().cursor), ...commands.pending()].sort((a,b) => String(a.data.command_sent_at ?? a.data.sent_at ?? '').localeCompare(String(b.data.command_sent_at ?? b.data.sent_at ?? '')) || String(a.data.command_id ?? a.id).localeCompare(String(b.data.command_id ?? b.id)) || Number(a.data.command_part_index ?? 0)-Number(b.data.command_part_index ?? 0)));
    const notice = () => commands.notice(chatID);
    const latest = () => commands.statuses().find(command => command.id === id());
    onCleanup(() => { thread.dispose(); commands.dispose(); });
    return <section>
        <label>Fixture message<input aria-label="Fixture message" value={text()} onInput={event => setText(event.currentTarget.value)}/></label>
        <input type="file" multiple aria-label="Fixture attachments" onChange={event => { const selected=Array.from(event.currentTarget.files ?? []); setOriginalFiles(selected); void stage(selected); }}/><output aria-label="Fixture ready attachments">{uploading() ? 'Uploading…' : String(files().length)}</output>
        <button data-compose-send disabled={uploading() || !commands.capabilities.allowed('messages.send')} onClick={event => {
            try { setID(commands.send(event, { chat_id: chatID, text: text(), ...(files().length ? { files: files() } : {}) })); setText(''); setFiles([]); }
            catch (error) { setError((error as Error).message); }
        }}>Send fixture message</button>
        <button disabled={!id() || live.status() !== 'connected'} onClick={() => commands.check(id())}>Check fixture command</button>
        <button disabled={!id()} onClick={event => { void commands.copy(event, id()); }}>Copy fixture message</button>
        <button disabled={!id()} onClick={event => { void commands.copy(event,id(),0); }}>Copy fixture text part</button>
        <button onClick={async () => {
            const file=originalFiles()[1]; if(!file)return;
            const response=await fetch('/web/v1/upload',{method:'POST',body:file}), value=await response.json();
            if(response.ok && value.state==='ready')setRetryFile({upload_id:value.upload_id,name:file.name});
        }}>Prepare fixture failed part</button>
        <output aria-label="Retry upload ready">{String(Boolean(retryFile()))}</output>
        <button onClick={event => {
            try { commands.retry(event,id(),2,retryFile()); }
            catch { setError('Fixture retry unavailable'); }
        }}>Retry fixture failed part</button>
        <button onClick={() => live.reconnect()}>Reconnect command fixture</button>
        <output aria-label="Fixture connection">{live.status()}</output>
        <output aria-label="Fixture ambiguity origin">{latest()?.transport_lost ? 'local transport' : latest()?.disposition ?? ''}</output>
        <output aria-label="Fixture retry allowed">{String(commands.canRetry(id()))}</output>
        <output aria-label="Fixture command ID">{id()}</output>
        <output aria-label="Fixture command status">{latest() ? messageCommandLabel(latest()!) : ''}</output>
        <output aria-label="Fixture captured sequence">{String(captured().cursor?.sequence ?? "")}</output>
        <output aria-label="Fixture captured count">{captured().items.length}</output>
        <output aria-label="Fixture live command count">{live.pending().length}</output>
        <output aria-label="Fixture owned command count">{commands.statuses().length}</output>
        <output aria-label="Fixture echo count">{commands.pending().length}</output>
        <output aria-label="Captured metadata untouched">{String((thread.data().items ?? []).every(item => item.data.command_id === undefined))}</output>
        <output aria-label="Projection timeout count">{commands.timeoutCount()}</output>
        <output aria-label="Missing observed identity count">{commands.missingIdentityCount()}</output>
        <For each={notice() ? [notice()!] : []}>{value => <aside aria-label="Uncertain delivery">{value.text} · <button disabled={live.status() !== 'connected'} onClick={() => value.command_ids.forEach(id => commands.check(id))}>Check</button></aside>}</For>
        <output role="alert">{error()}</output>
        <For each={rows()}>{item => <p data-record-id={item.id} data-send-state={String(item.data.send_state ?? '')} data-command-sent-at={String(item.data.command_sent_at ?? '')} data-native-sent-at={item.data.sent_at} data-parts={JSON.stringify(item.data.parts ?? [])} data-record-version={item.version} data-command-part-index={typeof item.data.command_part_index === 'number' ? item.data.command_part_index : undefined} data-command-id={String(item.data.command_id ?? '')} data-local-echo={item.id.startsWith('command:') ? '' : undefined}>{String(item.data.text ?? '')}</p>}</For>
    </section>;
}
