import { createSignal, onCleanup, untrack } from 'solid-js';
import { api, APIError } from '../../auth/api';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { array, fields, text, type Handle } from './types';
import { initials } from '../format';
export function createHandlesCache(live: LiveClient, capacity = 4096) {
    const [handles, setHandles] = createSignal<ReadonlyMap<string,Handle>>(new Map());
    const [revision,setRevision]=createSignal(0);
    const subscriptions = new Map<string, ReturnType<LiveClient['subscribe']>>();
    const resolved=new Set<string>(),inFlight=new Set<string>(),pending=new Set<string>();
    let frame = 0, unavailable=false, stopped=false, resolving=false;
    function evict(id:string){resolved.delete(id);subscriptions.get(id)?.dispose();subscriptions.delete(id);}
    function put(id: string, data: Record<string,unknown>) {
        const display = text(data.display_name ?? data.full_name ?? data.name ?? data.address);
        if (!id||!display) return;
        const avatar=fields(data.avatar),digest=text(avatar.digest);
        setHandles(previous => {
            const old=previous.get(id),contact_id=text(data.contact_id)||old?.contact_id;
            if(old?.display_name===display&&(!digest||old?.avatar?.digest===digest)&&old?.contact_id===contact_id)return previous;
            const next = new Map(previous);
            next.delete(id);
            next.set(id,{id,display_name:display,initials:text(data.initials)||initials(display),address:text(data.address)||old?.address,contact_id,avatar:digest?{digest,variant:text(avatar.variant)||'thumb'}:old?.avatar});
            while(next.size>capacity){const key=next.keys().next().value!;next.delete(key);evict(key);}
            return next;
        });
    }
    function seed(chats: readonly ChatSummary[]) {
        for (const chat of chats) for (const participant of array(chat.chat.data.participants)) { const data=fields(participant); put(text(data.handle_id ?? data.id),data); }
    }
    function fallback(ids:readonly string[]){
        if(stopped)return;
        for(const id of ids)if(!subscriptions.has(id)){
            while(subscriptions.size>=48){const key=subscriptions.keys().next().value!;evict(key);}
            subscriptions.set(id,live.subscribe('record',{collection:'messages.handle',id}));
        }
        setRevision(value=>value+1);
    }
    async function resolve(ids:string[]){
        try {
            if(unavailable){fallback(ids);return;}
            const result=await api<{handles:Record<string,unknown>[]}>('handles',{ids});
            if(stopped)return;
            for(const handle of result.handles){const id=text(handle.handle_id);put(id,handle);if(untrack(handles).has(id))resolved.add(id);}
        }catch(error){
            if(error instanceof APIError&&[400,404,501].includes(error.status))unavailable=true;
            fallback(ids);
        }finally{
            for(const id of ids)inFlight.delete(id);
            resolving=false;
            schedule();
        }
    }
    function schedule(){
        if(frame||resolving||stopped||!pending.size)return;
        frame=requestAnimationFrame(()=>{frame=0;const ids=[...pending].slice(0,500);for(const id of ids){pending.delete(id);inFlight.add(id);}resolving=true;void resolve(ids);});
    }
    function ensure(ids: readonly string[]) {
        for (const id of ids) if(id&&!resolved.has(id)&&!inFlight.has(id)&&!subscriptions.has(id))pending.add(id);
        schedule();
    }
    function read() {
        revision();
        for(const [id,subscription] of subscriptions) { const record=subscription.data().records?.[0];if(record){put(id,record.data);if(untrack(handles).has(id))resolved.add(id);} }
        return handles();
    }
    onCleanup(()=>{stopped=true;cancelAnimationFrame(frame);for(const subscription of subscriptions.values())subscription.dispose();});
    return {read,ensure,seed};
}
