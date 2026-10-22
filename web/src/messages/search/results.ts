import { createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import type { LiveClient } from '../../data/live';
export type SearchQuery={text:string;kind:string;from_handle_id:string;from_contact_id:string;unread:boolean};
export function createSearchResults(live:LiveClient,query:SearchQuery){
    const [history,setHistory]=createSignal<string[]>(['']),[index,setIndex]=createSignal(0),[view,setView]=createSignal<ReturnType<LiveClient['subscribe']>>(),[failed,setFailed]=createSignal(false);
    createEffect(()=>{const current=live.subscribe('search',{...query,after:history()[index()],limit:24},{cache:false,resetOnReconnect:true});setView(current);setFailed(false);const timer=setTimeout(()=>{if(!current.data().items)setFailed(true);},10000);onCleanup(()=>{clearTimeout(timer);current.dispose();});});
    const rows=createMemo(()=>[...(view()?.data().items??[])].sort((a,b)=>Date.parse(b.data.sent_at)-Date.parse(a.data.sent_at)||a.id.localeCompare(b.id)));
    const next=()=>view()?.data().next;
    const forward=()=>{const cursor=next();if(cursor){setHistory([...history().slice(0,index()+1),cursor]);setIndex(value=>value+1);}};
    return {rows,loading:()=>!view()?.data().items,failed,next,forward,previous:()=>setIndex(value=>Math.max(0,value-1)),hasPrevious:()=>index()>0,retry:()=>{setFailed(false);live.reconnect();}};
}
