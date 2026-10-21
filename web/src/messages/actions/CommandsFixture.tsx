import { batch, createEffect, createMemo, createSignal, For, onCleanup, untrack } from 'solid-js';
import { auth } from '../../auth/session';
import { createMessageCommands, messageCommandLabel } from './commands';
export function CommandsFixture() {
    const live = auth.live()!;
    const chatID = new URLSearchParams(location.search).get('chat') ?? '';
    const commands = createMessageCommands(live), thread = live.subscribe('thread', { chat_id: chatID }, { cache: false });
    const [text, setText] = createSignal(''), [id, setID] = createSignal(''), [error, setError] = createSignal('');
    const [captured, setCaptured] = createSignal({ items: thread.data().items ?? [], cursor: thread.cursor() });
    createEffect(() => { const items = thread.data().items ?? [], cursor = thread.cursor(); untrack(() => batch(() => { commands.observe(items, cursor); setCaptured({ items, cursor }); })); });
    const rows = createMemo(() => [...commands.projectCaptured(captured().items, captured().cursor), ...commands.pending()]);
    const latest = () => commands.statuses().find(command => command.id === id());
    onCleanup(() => { thread.dispose(); commands.dispose(); });
    return <section>
        <label>Fixture message<input aria-label="Fixture message" value={text()} onInput={event => setText(event.currentTarget.value)}/></label>
        <button data-compose-send disabled={!commands.capabilities.allowed('messages.send')} onClick={event => {
            try { setID(commands.send(event, { chat_id: chatID, text: text() })); setText(''); }
            catch (error) { setError((error as Error).message); }
        }}>Send fixture message</button>
        <button onClick={() => live.reconnect()}>Reconnect command fixture</button>
        <output aria-label="Fixture connection">{live.status()}</output>
        <output aria-label="Fixture ambiguity origin">{latest()?.transport_lost ? 'local transport' : latest()?.disposition ?? ''}</output>
        <output aria-label="Fixture command ID">{id()}</output>
        <output aria-label="Fixture command status">{latest() ? messageCommandLabel(latest()!) : ''}</output>
        <output aria-label="Captured metadata untouched">{String((thread.data().items ?? []).every(item => item.data.command_id === undefined))}</output>
        <output aria-label="Projection timeout count">{commands.timeoutCount()}</output>
        <output role="alert">{error()}</output>
        <For each={rows()}>{item => <p data-record-id={item.id} data-command-sent-at={String(item.data.command_sent_at ?? '')} data-native-sent-at={item.data.sent_at} data-parts={JSON.stringify(item.data.parts ?? [])} data-command-id={String(item.data.command_id ?? '')} data-local-echo={item.id.startsWith('command:') ? '' : undefined}>{String(item.data.text ?? '')}</p>}</For>
    </section>;
}
