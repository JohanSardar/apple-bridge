import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { ChatSummary, Cursor } from '../../data/types';
import { viewCache } from '../../data/cache';
import { strings as s } from '../../app/strings';
export type ChatFilter = 'all' | 'known' | 'unknown' | 'unread' | 'deleted' | 'spam';
export const chatFilters: { value: ChatFilter; label: string; empty: string }[] = [
    {value:'all',label:s.allMessages,empty:s.noMessages},
    {value:'known',label:s.knownSenders,empty:s.noKnownSenders},
    {value:'unknown',label:s.unknownSenders,empty:s.noUnknownSenders},
    {value:'unread',label:s.unread,empty:s.noUnreadMessages},
    {value:'deleted',label:s.recentlyDeleted,empty:s.noDeletedMessages},
    {value:'spam',label:s.spam,empty:s.noSpam}
];
function boundedSnapshot(rows:ChatSummary[]){
    const chats:ChatSummary[]=[],encoder=new TextEncoder();let bytes=2;
    for(const row of rows.slice(0,2000)){const size=encoder.encode(JSON.stringify(row)).byteLength+1;if(bytes+size>2*1024*1024-1024)break;chats.push(row);bytes+=size;}
    return {chats,snapshot_version:1,snapshot_complete:chats.length===rows.length};
}
function createChatView(live: LiveClient, filter: Accessor<ChatFilter | undefined>) {
    type Page={after?:string;view:ReturnType<LiveClient['subscribe']>};
    const [pages,setPages]=createSignal<Page[]>([]),[delayed,setDelayed]=createSignal(false);
    const [ready,setReady]=createSignal(false);
    let complete:ChatSummary[]=[],completeKnown=false,cachedCursor:Cursor|undefined;
    createEffect(()=>{
        const value=filter();
        complete=[];completeKnown=false;cachedCursor=undefined;setReady(false);setPages([]);setDelayed(false);
        if(!value){setReady(true);return;}
        let active=true;
        const timer=setTimeout(()=>setDelayed(true),150);
        void viewCache.read(JSON.stringify(['chats-snapshot',1,value])).then(cached=>{
            if(!active)return;
            if(cached?.data.snapshot_version===1&&cached.data.chats){complete=cached.data.chats;completeKnown=true;cachedCursor=cached.cursor;}
        }).catch(()=>{}).then(()=>{
            if(!active)return;
            setPages([{view:live.subscribe('chats',{filter:value,limit:200},{cache:false,cursor:cachedCursor})}]);setReady(true);
        });
        onCleanup(()=>{active=false;clearTimeout(timer);for(const page of pages())page.view.dispose();});
    });
    createEffect(()=>{
        const current=pages();
        for(let index=0;index<current.length;index++){
            const data=current[index].view.data();
            if(!data.chats)continue;
            const after=data.next;
            if(after===current[index+1]?.after)continue;
            const tail=current.slice(index+1);
            tail.forEach(page=>page.view.dispose());
            setPages([...current.slice(0,index+1),...(after?[{after,view:live.subscribe('chats',{filter:filter(),limit:200,after},{cache:false})}]:[])]);
            break;
        }
    });
    const rows=createMemo(()=>{
        if(!ready())return [];
        const unique=new Map<string,ChatSummary>();
        for(const page of pages())for(const chat of page.view.data().chats??[])if(chat.chat?.id&&!chat.chat.data.is_archived&&!unique.has(chat.chat.id)){const pinned=chat.chat.data.pinned_index;unique.set(chat.chat.id,chat.pinned_index==null&&typeof pinned==='number'?{...chat,pinned_index:pinned}:chat);}
        const current=pages(),last=current.at(-1)?.view.data();
        const cursor=current[0]?.view.cursor();
        const pending=current.some(page=>{const next=page.view.cursor();return !cursor||!next||next.epoch!==cursor.epoch||next.sequence!==cursor.sequence;})||current.some((page,index)=>!page.view.data().chats||page.view.data().next!==current[index+1]?.after)||Boolean(last?.next);
        if(pending)for(const chat of complete)if(!unique.has(chat.chat.id))unique.set(chat.chat.id,chat);
        const ordered=[...unique.values()].sort((a,b)=>(Date.parse(b.last_activity_at)||0)-(Date.parse(a.last_activity_at)||0)||a.chat.id.localeCompare(b.chat.id));
        if(!pending){complete=ordered;completeKnown=true;}
        return ordered;
    });
    createEffect(()=>{
        const value=filter(),current=rows(),cursor=pages()[0]?.view.cursor()??cachedCursor;
        if(value&&ready()&&completeKnown)void viewCache.write(JSON.stringify(['chats-snapshot',1,value]),boundedSnapshot(current),cursor).catch(()=>{});
    });
    return {rows,loading:()=>{rows();return !ready()||(!completeKnown&&!pages()[0]?.view.data().chats);},delayed};
}
export function createChatsStore(live: LiveClient) {
    const [filter,setFilter]=createSignal<ChatFilter>('all');
    const all=createChatView(live,()=> 'all');
    const filtered=createChatView(live,()=>filter()==='all'?undefined:filter());
    const visibleAll=createMemo(()=>all.rows().filter(chat=>!chat.chat.data.deleted_at));
    const cachedFilter=createMemo(()=>{
        const value=filter();
        return all.rows().filter(chat=>{
            const data=chat.chat.data;
            if(value==='deleted')return Boolean(data.deleted_at);
            if(data.deleted_at)return false;
            if(value==='unread')return chat.unread_count>0;
            const linked=(chat.avatar_handles??[]).some(handle=>Boolean(handle.contact_id));
            if(value==='known')return (data.filter??'known')==='known'||linked;
            if(value==='unknown')return data.filter==='unknown'&&!linked;
            if(value==='spam')return data.is_spam===true;
            return true;
        });
    });
    const rows=()=>filter()==='all'?visibleAll():filtered.loading()?cachedFilter():filtered.rows();
    const unread=createMemo(()=>visibleAll().filter(row=>row.unread_count>0).length);
    return {all:visibleAll,rows,filter,setFilter,unread,loading:()=>filter()==='all'?all.loading():filtered.loading()&&cachedFilter().length===0,delayed:()=>filter()==='all'?all.delayed():filtered.delayed()};
}
export type ChatsStore=ReturnType<typeof createChatsStore>;
