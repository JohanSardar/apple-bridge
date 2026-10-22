import { createEffect, createSignal, For, onCleanup, Show, untrack } from 'solid-js';
import { auth } from '../../auth/session';
import { withInput } from '../../data/input';
import { createCapabilities } from './capabilities';
import { createMessageActions } from './reactionActions';
import { MessageActions } from './MessageActions';
export function MessageActionsFixture() {
    const live = auth.live()!, chatID = new URLSearchParams(location.search).get('chat') ?? '';
    const capabilities = createCapabilities(live);
    const actions=createMessageActions(live,capabilities);
    const thread = live.subscribe('thread',{chat_id:chatID},{cache:false});
    const [created,setCreated] = createSignal('');
    createEffect(()=>{const items=thread.data().items??[],cursor=thread.cursor();untrack(()=>actions.observe(items,cursor,chatID));});
    onCleanup(() => {thread.dispose();capabilities.dispose();});
    return <section aria-label="Message actions fixture">
        <button disabled={!capabilities.allowed('messages.send')} onClick={event => setCreated(withInput(event,() => live.command('messages.send',{chat_id:chatID,text:'[ab-test actions] fresh self message'})))}>Create self fixture message</button>
        <output aria-label="Created fixture message">{live.pending().find(command => command.id === created())?.observed_record_ids?.[0] ?? ''}</output>
        <For each={thread.data().items ?? []}>{item => <button class="action-fixture-message" data-message={item.id} data-version={item.version} data-sent-at={item.data.sent_at} onContextMenu={event=>actions.open(event,item)}>{String(item.data.text ?? 'Attachment')}</button>}</For>
        <MessageActions actions={actions}/>
        <Show when={actions.notice(chatID)}>{notice=><aside role="status" aria-label="Reaction status">{notice().text}</aside>}</Show>
    </section>;
}
