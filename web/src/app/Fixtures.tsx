import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { VirtualList, type VirtualListHandle } from '../virtual/VirtualList';
import { auth } from '../auth/session';
import { withInput } from '../data/input';
import { springAnimation, sharedElement, reducedMotion } from '../motion/animate';
import { router } from './router';
type Row = {
    id: string;
    text: string;
};
export default function Fixtures() {
    const kind = () => router.route().path.split('/').at(-1);
    const [status, setStatus] = createSignal('Ready');
    const [observed,setObserved]=createSignal(0);
    const [rows, setRows] = createSignal<Row[]>(kind() === 'virtual' || kind() === 'perf' ? Array.from({ length: 100000 }, (_, i) => ({ id: String(i), text: `Row ${i} · ` + 'Variable height content. '.repeat(i % 7 + 1) })) : []);
    const [bottom, setBottom] = createSignal(false);
    let handle: VirtualListHandle | undefined;
    const live = auth.live();
    const subscription = live?.subscribe('chats', { filter: 'all' });
    createEffect(()=>{ subscription?.data(); setObserved(Date.now()); });
    onCleanup(() => subscription?.dispose());
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
    return <main class="fixture"><header><button class="button subtle" onClick={() => router.navigate('/messages')}>Back to shell</button><h1>Foundation fixture</h1></header><Show when={kind() === 'input'}><button class="button" onClick={command}>Submit fixture command</button><button class="button" onClick={event => { setTimeout(() => command(event), 0); }}>Delayed fixture command</button></Show><Show when={kind() === 'virtual' || kind() === 'perf'}><header><button class="button subtle" onClick={() => { setRows(previous => [...Array.from({ length: 20 }, (_, i) => ({ id: 'older-' + i, text: 'Older row ' + i })), ...previous]); }}>Prepend 20</button><button class="button subtle" onClick={() => setRows(previous => [...previous, { id: 'appended-' + previous.length, text: 'Appended row' }])}>Append</button><button class="button subtle" onClick={() => handle?.scrollToIndex(50000)}>Middle</button><button class="button subtle" onClick={() => { setBottom(!bottom()); handle?.scrollToIndex(rows().length - 1, 'end'); }}>Bottom mode</button></header><VirtualList items={rows()} keyFor={row => row.id} estimate={row => 48 + Number(row.id) % 7 * 16} bottomAnchored={bottom()} label="100000 variable-height rows" ref={value => { handle = value; }} render={row => <div class="fixture-row">{row.text}</div>}/></Show><Show when={kind() === 'live'}><button class="button subtle" onClick={()=>live?.reconnect()}>Reconnect live view</button><output aria-label="View observed at">{observed()}</output><output aria-label="Unread count">{subscription?.data().chats?.reduce((sum,row)=>sum+(row.unread_count??0),0)??0}</output><output aria-label="Latest message">{subscription?.data().chats?.[0]?.last_message?.id??''}</output><output aria-label="Live count">{subscription?.data().chats?.length ?? 0}</output><For each={subscription?.data().chats ?? []}>{row => <p>{row.title}</p>}</For><p role="status">{live?.status()}</p></Show><Show when={kind() === 'motion'}><button class="button" onClick={() => { if (reducedMotion()) {
        animated.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 150 });
        return;
    } animation?.cancel(); animation = springAnimation(value => { animated.style.transform = `translateX(${value}px)`; }, 0, 180, { response: .42, damping: .82 }, 0, () => setStatus('Settled')); }}>Animate spring</button><button class="button" onClick={() => animation?.retarget(0)}>Interrupt spring</button><button class="button" onClick={() => { cancelFlip?.(); cancelFlip = sharedElement(animated, () => { expanded = !expanded; animated.style.width = expanded ? '200px' : '80px'; animated.style.height = expanded ? '200px' : '80px'; }); }}>Shared element</button><div ref={animated} class="motion-fixture" style={{ width: '80px', height: '80px', background: 'var(--blue)', 'border-radius': '20px' }}/></Show><Show when={kind() !== 'live'}><output role="status">{status()}</output></Show></main>;
}
