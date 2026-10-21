import { batch, createEffect, createMemo, createSignal, For, onCleanup, Show, untrack } from 'solid-js';
import { auth, endSession } from '../../auth/session';
import type { LiveClient } from '../../data/live';
import { createMessageCommands } from '../actions/commands';
import { Composer } from './Composer';
export function ComposeFixture() {
    const live = auth.live()!, commands = createMessageCommands(live), query = new URLSearchParams(location.search);
    const first = query.get('chat') ?? '', alternate = query.get('alternate') ?? '';
    const [chatID, setChatID] = createSignal(first);
    return <section><button class="button" onClick={() => setChatID(chatID() === first ? alternate : first)}>Switch fixture chat</button><button class="button" onClick={() => { void endSession(); }}>Sign out</button><Show when={chatID()} keyed>{id => <ComposeChat live={live} commands={commands} chatID={id}/>}</Show></section>;
}
function ComposeChat(props: { live: LiveClient; commands: ReturnType<typeof createMessageCommands>; chatID: string }) {
    const thread = props.live.subscribe('thread', { chat_id: props.chatID }, { cache: false, resetOnReconnect: true });
    const chat = props.live.subscribe('record', { collection: 'messages.chat', id: props.chatID }, { cache: false });
    const [captured, setCaptured] = createSignal({ items: thread.data().items ?? [], cursor: thread.cursor() });
    createEffect(() => {
        const items = thread.data().items ?? [], cursor = thread.cursor();
        untrack(() => batch(() => { props.commands.observe(items, cursor); setCaptured({ items, cursor }); }));
    });
    const rows = createMemo(() => [...props.commands.projectCaptured(captured().items, captured().cursor), ...props.commands.pending().filter(item => item.data.chat_id === props.chatID)]);
    const latest = () => props.commands.statuses().filter(command => (command.args as { chat_id?: string }).chat_id === props.chatID).at(-1);
    onCleanup(() => { thread.dispose(); chat.dispose(); });
    return <div>
        <For each={rows()}>{item => <p data-record-id={item.id} data-local-echo={item.id.startsWith('command:') ? '' : undefined}>{String(item.data.text ?? '')}</p>}</For>
        <Composer draftID={props.chatID} service={String(chat.data().records?.[0]?.data.service ?? '')} session={auth.session()!} available={Boolean(captured().cursor) && props.commands.capabilities.allowed('messages.send')} onSend={(event, content) => { if (new URLSearchParams(location.search).has('reject')) throw new Error('internal_fixture_rejection'); return props.commands.send(event, { chat_id: props.chatID, ...content }); }} command={latest()} onCheck={() => { const command = latest(); if (command) props.commands.check(command.id); }} onCopy={event => { const command = latest(); if (command) void props.commands.copy(event, command.id); }}/>
    </div>;
}
