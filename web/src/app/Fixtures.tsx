import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { VirtualList, type VirtualListHandle } from '../virtual/VirtualList';
import { auth } from '../auth/session';
import { viewCache } from '../data/cache';
import { withInput } from '../data/input';
import { springAnimation, sharedElement, reducedMotion } from '../motion/animate';
import { router } from './router';
import { MediaFixture } from '../messages/media/MediaFixture';
import { SourcesFixture } from '../messages/actions/SourcesFixture';
import { MessageActionsFixture } from '../messages/actions/MessageActionsFixture';
import { CommandsFixture } from '../messages/actions/CommandsFixture';
import { TextSizingFixture } from '../messages/transcript/TextSizingFixture';
import {PointerFixture} from '../messages/transcript/PointerFixture';
import {ReplyHandoffFixture} from '../messages/transcript/ReplyHandoffFixture';
import { HooksFixture } from '../messages/transcript/HooksFixture';
import { ComposeFixture } from '../messages/compose/ComposeFixture';
import { createChatsStore, chatFilters, type ChatFilter } from '../messages/store/chats';
type Row = {
    id: string;
    text: string;
};
export default function Fixtures() {
    const kind = () => new URL(router.route().path,location.origin).pathname.split('/').at(-1);
    const [status, setStatus] = createSignal('Ready');
    const [observed,setObserved]=createSignal(0);
    const [rows, setRows] = createSignal<Row[]>(kind() === 'virtual' || kind() === 'perf' ? Array.from({ length: 100000 }, (_, i) => ({ id: String(i), text: `Row ${i} · ` + 'Variable height content. '.repeat(i % 7 + 1) })) : []);
    const [bottom, setBottom] = createSignal(false);
    let handle: VirtualListHandle | undefined;
    const live = auth.live();
    const chats=live?createChatsStore(live):undefined;
    const chatRows=()=>chats?.rows()??[];
    createEffect(()=>{ chatRows(); setObserved(Date.now()); });
    const command = (event: Event) => { try {
        withInput(event, () => live?.command('fixture.command', {}));
        setStatus('Submitted');
    }
    catch (error) {
        setStatus((error as Error).message);
    } };
    createEffect(() => { const commands = live?.pending(); if (commands?.length)
        setStatus(commands.at(-1)!.status); });
    let animated!: HTMLDivElement;
    let animation: ReturnType<typeof springAnimation> | undefined;
    let cancelFlip: (() => void) | undefined;
    let expanded = false;
    onCleanup(() => { animation?.cancel(); cancelFlip?.(); });
    return <main class="fixture"><header><button class="button subtle" onClick={() => router.navigate('/messages')}>Back to shell</button><h1>Foundation fixture</h1></header><Show when={kind()==='transcript-pointer'}><PointerFixture/></Show><Show when={kind()==='reply-handoff'}><ReplyHandoffFixture/></Show><Show when={kind() === 'cache'}><button class="button" onClick={()=>{void viewCache.write('eviction-new',{records:[]}).then(()=>setStatus('Persisted'));}}>Persist fixture view</button></Show><Show when={kind() === 'transcript-hooks'}><HooksFixture/></Show><Show when={kind() === 'text-sizing'}><TextSizingFixture/></Show><Show when={kind() === 'compose'}><ComposeFixture/></Show><Show when={kind() === 'message-actions'}><MessageActionsFixture/></Show><Show when={kind() === 'commands'}><CommandsFixture/></Show><Show when={kind() === 'sources'}><SourcesFixture/></Show><Show when={kind() === 'media'}><MediaFixture/></Show><Show when={kind() === 'input'}><button class="button" onClick={command}>Submit fixture command</button><button class="button" onClick={event => { setTimeout(() => command(event), 0); }}>Delayed fixture command</button><button class="button" onClick={()=>command({isTrusted:true,eventPhase:Event.BUBBLING_PHASE,type:'click'} as Event)}>Forged fixture context</button><label><input type="checkbox" aria-label="Programmatic change" onChange={command}/>Programmatic change</label></Show><Show when={kind() === 'virtual' || kind() === 'perf'}><header><button class="button subtle" onClick={() => { setRows(previous => [...Array.from({ length: 20 }, (_, i) => ({ id: 'older-' + i, text: 'Older row ' + i })), ...previous]); }}>Prepend 20</button><button class="button subtle" onClick={() => setRows(previous => [...previous, { id: 'appended-' + previous.length, text: 'Appended row' }])}>Append</button><button class="button subtle" onClick={() => handle?.scrollToIndex(50000)}>Middle</button><button class="button subtle" onClick={() => { setBottom(!bottom()); handle?.scrollToIndex(rows().length - 1, 'end'); }}>Bottom mode</button></header><VirtualList items={rows()} keyFor={row => row.id} estimate={row => 48 + (Number(row.id)||0) % 7 * 16} bottomAnchored={bottom()} label="100000 variable-height rows" ref={value => { handle = value; }} render={row => <div class="fixture-row">{row.text}</div>}/></Show><Show when={kind() === 'live'}><label>Chat filter<select aria-label="Chat filter" value={chats?.filter()} onChange={event=>chats?.setFilter(event.currentTarget.value as ChatFilter)}><For each={chatFilters}>{filter=><option value={filter.value}>{filter.label}</option>}</For></select></label><output aria-label="Unread chats">{chats?.unread()??0}</output><button class="button subtle" onClick={()=>live?.reconnect()}>Reconnect live view</button><output aria-label="View observed at">{observed()}</output><output aria-label="Unread count">{chatRows().reduce((sum,row)=>sum+(row.unread_count??0),0)??0}</output><output aria-label="Latest message">{chatRows()[0]?.last_message?.id??''}</output><output aria-label="Latest preview">{chatRows()[0]?.last_message?.preview.text??''}</output><output aria-label="Live count">{chatRows().length ?? 0}</output><For each={chatRows()}>{row => <p data-chat={row.chat.id}>{row.title}</p>}</For><p role="status" aria-label="Connection">{live?.status()}</p></Show><Show when={kind() === 'motion'}><button class="button" onClick={() => { if (reducedMotion()) {
        animated.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 150 });
        return;
    } animation?.cancel(); animation = springAnimation(value => { animated.style.transform = `translateX(${value}px)`; }, 0, 180, { response: .42, damping: .82 }, 0, () => setStatus('Settled')); }}>Animate spring</button><button class="button" onClick={() => animation?.retarget(0)}>Interrupt spring</button><button class="button" onClick={() => { cancelFlip?.(); cancelFlip = sharedElement(animated, () => { expanded = !expanded; animated.style.width = expanded ? '200px' : '80px'; animated.style.height = expanded ? '200px' : '80px'; }); }}>Shared element</button><div ref={animated} class="motion-fixture" style={{ width: '80px', height: '80px', background: 'var(--blue)', 'border-radius': '20px' }}/></Show><Show when={kind() !== 'live'}><output role="status">{status()}</output></Show></main>;
}
