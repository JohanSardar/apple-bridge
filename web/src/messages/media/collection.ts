import { createEffect, createMemo, createSignal, onCleanup, untrack } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord } from '../../data/types';
export function createMediaCollection(live:LiveClient,chatId:string,pageSize=200){
    type Page=ReturnType<LiveClient['subscribe']>;
    const head=live.subscribe('collection',{chat_id:chatId,kind:'media',limit:pageSize});
    const [page,setPage]=createSignal<Page>(),[rows,setRows]=createSignal<BridgeRecord[]>([]),[loading,setLoading]=createSignal(true),[error,setError]=createSignal(false);
    let scan=new Map<string,BridgeRecord>(),seen=new Set<string>(),revision='',scanning=false,pending=false,disposed=false,timer:ReturnType<typeof setTimeout>,timeout:ReturnType<typeof setTimeout>;
    let headTimeout=setTimeout(()=>{setError(true);setLoading(false);},10000);
    const merge=(values:readonly BridgeRecord[])=>setRows(previous=>[...new Map([...previous,...values].map(value=>[value.id,value])).values()]);
    const finish=()=>{setError(false);scanning=false;setLoading(false);setRows([...new Map([...scan.values(),...(head.data().items??[])].map(value=>[value.id,value])).values()]);if(pending){pending=false;timer=setTimeout(start,150);}};
    const next=(after?:string)=>{
        if(!after){finish();return;}
        if(seen.has(after)){setError(true);setLoading(false);scanning=false;return;}
        seen.add(after);
        timer=setTimeout(()=>{if(disposed)return;setPage(live.subscribe('collection',{chat_id:chatId,kind:'media',limit:pageSize,after}));timeout=setTimeout(()=>{setError(true);setLoading(false);},10000);},100);
    };
    const start=()=>{if(disposed)return;if(scanning){pending=true;return;}const data=head.data();if(!data.items)return;clearTimeout(timer);setError(false);setLoading(true);scanning=true;scan=new Map(data.items.map(value=>[value.id,value]));seen=new Set();next(data.next);};
    createEffect(()=>{const data=head.data(),cursor=head.cursor();if(!data.items)return;clearTimeout(headTimeout);const key=`${cursor?.epoch}:${cursor?.sequence}`;untrack(()=>{merge(data.items!);if(revision!==key){revision=key;start();}});});
    createEffect(()=>{const current=page(),data=current?.data();if(!current||!data?.items)return;untrack(()=>{clearTimeout(timeout);setError(false);for(const item of data.items!)scan.set(item.id,item);merge(data.items!);current.dispose();setPage(undefined);next(data.next);});});
    onCleanup(()=>{disposed=true;clearTimeout(headTimeout);clearTimeout(timer);clearTimeout(timeout);head.dispose();page()?.dispose();});
    const attachments=createMemo(()=>rows().filter(value=>/^(image|video)\//u.test(String(value.data.mime_type))&&!value.data.is_sticker&&!value.data.genmoji&&!value.data.hidden).sort((a,b)=>Date.parse(String(a.data.sent_at))-Date.parse(String(b.data.sent_at))||a.id.localeCompare(b.id)));
    return {attachments,loading,error,retry:()=>{page()?.dispose();setPage(undefined);clearTimeout(timer);clearTimeout(timeout);scanning=false;pending=false;if(!head.data().items){setError(false);setLoading(true);live.reconnect();clearTimeout(headTimeout);headTimeout=setTimeout(()=>{setError(true);setLoading(false);},10000);}else start();}};
}
