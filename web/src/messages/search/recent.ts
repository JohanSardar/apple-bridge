import { createSignal } from 'solid-js';
import type { LiveClient } from '../../data/live';
import { viewCache } from '../../data/cache';
const memory=new WeakMap<LiveClient,string[]>(),key=JSON.stringify(['messages-search-recents',1]);
export function createRecentSearches(live:LiveClient,persistent:boolean){
    const [read,setRead]=createSignal(memory.get(live)??[]);let revision=0;
    const merge=(values:string[])=>[...new Set(values.map(value=>value.trim()).filter(value=>value&&value.length<=512))].slice(0,8);
    if(persistent)void viewCache.read(key).then(value=>{if(revision)return;const entries=value?.data.recent_searches;if(Array.isArray(entries)){const next=merge([...read(),...entries.filter((entry):entry is string=>typeof entry==='string')]);memory.set(live,next);setRead(next);}}).catch(()=>{});
    const remember=(value:string)=>{revision++;const next=merge([value,...read()]);memory.set(live,next);setRead(next);if(persistent)void viewCache.write(key,{recent_searches:next}).catch(()=>{});};
    const clear=()=>{revision++;memory.set(live,[]);setRead([]);if(persistent)void viewCache.write(key,{recent_searches:[]}).catch(()=>{});};
    return {read,remember,clear};
}
