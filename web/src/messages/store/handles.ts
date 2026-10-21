import { createSignal, onCleanup } from 'solid-js';
import { api, APIError } from '../../auth/api';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { array, fields, text, type Handle } from './types';
import { initials } from '../format';
export function createHandlesCache(live: LiveClient) {
    const [handles, setHandles] = createSignal<ReadonlyMap<string,Handle>>(new Map());
    const [revision,setRevision]=createSignal(0);
    const subscriptions = new Map<string, ReturnType<LiveClient['subscribe']>>();
    const requested=new Set<string>();
    let pending = new Set<string>(), frame = 0, unavailable=false, stopped=false;
    function put(id: string, data: Record<string,unknown>) {
        const display = text(data.display_name ?? data.full_name ?? data.name ?? data.address);
        if (!id||!display) return;
        const avatar=fields(data.avatar),digest=text(avatar.digest),contact_id=text(data.contact_id);
        setHandles(previous => {
            const old=previous.get(id);
            if(old?.display_name===display&&old?.avatar?.digest===digest&&old?.contact_id===contact_id)return previous;
            const next = new Map(previous);
            next.delete(id);
            next.set(id,{id,display_name:display,initials:text(data.initials)||initials(display),address:text(data.address),contact_id,avatar:digest?{digest,variant:text(avatar.variant)||'thumb'}:undefined});
            while(next.size>500)next.delete(next.keys().next().value!);
            return next;
        });
    }
    function seed(chats: readonly ChatSummary[]) {
        for (const chat of chats) for (const participant of array(chat.chat.data.participants)) { const data=fields(participant); put(text(data.handle_id ?? data.id),data); }
    }
    function fallback(ids:readonly string[]){
        if(stopped)return;
        for(const id of ids)if(!subscriptions.has(id)){
            while(subscriptions.size>=500){const key=subscriptions.keys().next().value!;subscriptions.get(key)?.dispose();subscriptions.delete(key);}
            subscriptions.set(id,live.subscribe('record',{collection:'messages.handle',id}));
        }
        setRevision(value=>value+1);
    }
    async function resolve(ids:string[]){
        if(unavailable){fallback(ids);return;}
        try {
            const result=await api<{handles:Record<string,unknown>[]}>('handles',{ids});
            if(stopped)return;
            for(const handle of result.handles)put(text(handle.handle_id),handle);
        }catch(error){
            if(error instanceof APIError&&[400,404,501].includes(error.status))unavailable=true;
            fallback(ids);
        }
    }
    function ensure(ids: readonly string[]) {
        for (const id of ids) if(id&&!requested.has(id)){pending.add(id);requested.add(id);}
        if(frame || !pending.size)return;
        frame=requestAnimationFrame(()=>{frame=0;const ids=[...pending].slice(0,500);pending=new Set();void resolve(ids);});
    }
    function read() {
        revision();
        for(const [id,subscription] of subscriptions) { const record=subscription.data().records?.[0];if(record)put(id,record.data); }
        return handles();
    }
    onCleanup(()=>{stopped=true;cancelAnimationFrame(frame);for(const subscription of subscriptions.values())subscription.dispose();});
    return {read,ensure,seed};
}
