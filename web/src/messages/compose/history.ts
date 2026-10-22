import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
export function createRecipientHistory(live: LiveClient, participants: Accessor<readonly string[]>) {
    const [rows,setRows] = createSignal<readonly ChatSummary[]>([]), [ready,setReady] = createSignal(false), [failed,setFailed] = createSignal(false);
    const handles = createMemo(() => [...participants()]);
    createEffect(() => {
        const values = handles();
        setRows([]); setReady(false); setFailed(false);
        if (!values.length) return;
        const view = live.subscribe('chats',{participants:values,limit:1},{cache:false,resetOnReconnect:true});
        createEffect(() => {
            const connected = live.status() === 'connected', data = view.data(), cursor = view.cursor();
            if (!connected) { setRows([]); setReady(false); return; }
            if (cursor && data.chats) { setRows(data.chats); setReady(true); setFailed(false); return; }
            const timer = setTimeout(() => setFailed(true),2000);
            onCleanup(() => clearTimeout(timer));
        });
        onCleanup(view.dispose);
    });
    return {rows,ready,failed};
}
