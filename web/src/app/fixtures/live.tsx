import { createEffect, createSignal, For } from 'solid-js';
import { auth } from '../../auth/session';
import { createChatsStore, chatFilters, type ChatFilter } from '../../messages/store/chats';
import { FixtureFrame } from '../FixtureFrame';
export default function LiveFixture() {
    const live = auth.live(), chats = live ? createChatsStore(live) : undefined;
    const rows = () => chats?.rows() ?? [];
    const [observed, setObserved] = createSignal(0);
    createEffect(() => { rows(); setObserved(Date.now()); });
    return <FixtureFrame live><label>Chat filter<select aria-label="Chat filter" value={chats?.filter()} onChange={event => chats?.setFilter(event.currentTarget.value as ChatFilter)}><For each={chatFilters}>{filter => <option value={filter.value}>{filter.label}</option>}</For></select></label><output aria-label="Unread chats">{chats?.unread() ?? 0}</output><button class="button subtle" onClick={() => live?.reconnect()}>Reconnect live view</button><output aria-label="View observed at">{observed()}</output><output aria-label="Unread count">{rows().reduce((sum, row) => sum + (row.unread_count ?? 0), 0)}</output><output aria-label="Latest message">{rows()[0]?.last_message?.id ?? ''}</output><output aria-label="Latest preview">{rows()[0]?.last_message?.preview.text ?? ''}</output><output aria-label="Live count">{rows().length}</output><For each={rows()}>{row => <p data-chat={row.chat.id}>{row.title}</p>}</For><p role="status" aria-label="Connection">{live?.status()}</p></FixtureFrame>;
}
