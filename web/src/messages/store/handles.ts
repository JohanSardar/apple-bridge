import { createRoot, createSignal, onCleanup, untrack } from 'solid-js';
import { api, APIError } from '../../auth/api';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { array, fields, text, type Handle } from './types';
import { initials } from '../format';
const caches=new WeakMap<LiveClient,{cache:ReturnType<typeof buildCache>;dispose:()=>void;users:number}>();
export function createHandlesCache(live:LiveClient,capacity=4096){
    let entry=caches.get(live);
    if(!entry){
        let dispose=()=>{};
        const cache=createRoot(release=>{dispose=release;return buildCache(live,capacity);});
        entry={cache,dispose,users:0};
        caches.set(live,entry);
    }
    const shared=entry;
    shared.users++;
    onCleanup(()=>{shared.users--;if(!shared.users){shared.dispose();caches.delete(live);}});
    return shared.cache;
}
function buildCache(live:LiveClient,capacity:number) {
    const [handles, setHandles] = createSignal<ReadonlyMap<string,Handle>>(new Map());
    const [revision,setRevision]=createSignal(0);
    const subscriptions = new Map<string, ReturnType<LiveClient['subscribe']>>();
    const resolved=new Set<string>(),inFlight=new Set<string>(),pending=new Set<string>(),fallbackPending=new Set<string>();
    let retry:ReturnType<typeof setTimeout>|undefined;
    let frame = 0, fallbackFrame=0, unavailable=false, stopped=false, resolving=false;
    function evict(id:string){resolved.delete(id);subscriptions.get(id)?.dispose();subscriptions.delete(id);}
    function put(id: string, data: Record<string,unknown>,confirmed=false) {
        const display = text(data.display_name ?? data.full_name ?? data.name ?? data.address);
        if (!id||!display) return;
        const avatar=fields(data.avatar),digest=text(avatar.digest);
        setHandles(previous => {
            const old=previous.get(id),contact_id=text(data.contact_id)||old?.contact_id,given_name=text(data.given_name)||old?.given_name,family_name=text(data.family_name)||old?.family_name,name_resolved=confirmed||old?.name_resolved===true;
            if(old?.display_name===display&&(!digest||old?.avatar?.digest===digest)&&old?.contact_id===contact_id&&old?.given_name===given_name&&old?.family_name===family_name&&old?.name_resolved===name_resolved)return previous;
            const next = new Map(previous);
            next.delete(id);
            next.set(id,{id,name_resolved,display_name:display,given_name,family_name,initials:text(data.initials)||initials(display),address:text(data.address)||old?.address,contact_id,avatar:digest?{digest,variant:text(avatar.variant)||'thumb'}:old?.avatar});
            while(next.size>capacity){const key=next.keys().next().value!;next.delete(key);evict(key);}
            return next;
        });
    }
    function seed(chats: readonly ChatSummary[]) {
        for (const chat of chats) for (const participant of array(chat.chat.data.participants)) { const data=fields(participant); put(text(data.handle_id ?? data.id),data); }
    }
    function scheduleFallback(){
        if(fallbackFrame||stopped||!fallbackPending.size||subscriptions.size>=48)return;
        fallbackFrame=requestAnimationFrame(()=>{
            fallbackFrame=0;
            for(const id of fallbackPending){
                if(subscriptions.size>=48)break;
                fallbackPending.delete(id);
                subscriptions.set(id,live.subscribe('record',{collection:'messages.handle',id}));
            }
            setRevision(value=>value+1);
        });
    }
    function fallback(ids:readonly string[]){
        if(stopped)return;
        for(const id of ids)if(!subscriptions.has(id)&&!resolved.has(id))fallbackPending.add(id);
        scheduleFallback();
    }
    async function resolve(ids:string[]){
        try {
            if(unavailable){fallback(ids);return;}
            const result=await api<{handles:Record<string,unknown>[]}>('handles',{ids});
            if(stopped)return;
            for(const handle of result.handles){const id=text(handle.handle_id);put(id,handle,true);if(untrack(handles).has(id))resolved.add(id);}
        }catch(error){
            if(stopped)return;
            if(error instanceof APIError&&error.status===429){
                for(const id of ids)pending.add(id);
                retry=setTimeout(()=>{retry=undefined;schedule();},1000);
            }else{
                if(error instanceof APIError&&[400,404,501].includes(error.status))unavailable=true;
                fallback(ids);
            }
        }finally{
            for(const id of ids)inFlight.delete(id);
            resolving=false;
            schedule();
        }
    }
    function schedule(){
        if(frame||retry||resolving||stopped||!pending.size)return;
        frame=requestAnimationFrame(()=>{frame=0;const ids=[...pending].slice(0,500);for(const id of ids){pending.delete(id);inFlight.add(id);}resolving=true;void resolve(ids);});
    }
    function ensure(ids: readonly string[]) {
        for (const id of ids) if(id&&!resolved.has(id)&&!inFlight.has(id)&&!subscriptions.has(id)&&!fallbackPending.has(id))pending.add(id);
        schedule();
    }
    function read() {
        revision();
        for(const [id,subscription] of subscriptions){const data=subscription.data();if(data.records===undefined)continue;const record=data.records[0];if(record)put(id,record.data,true);resolved.add(id);subscription.dispose();subscriptions.delete(id);}
        scheduleFallback();
        return handles();
    }
    onCleanup(()=>{stopped=true;clearTimeout(retry);cancelAnimationFrame(frame);cancelAnimationFrame(fallbackFrame);for(const subscription of subscriptions.values())subscription.dispose();});
    return {read,ensure,seed};
}
