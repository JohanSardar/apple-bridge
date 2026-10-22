import { createEffect, createMemo, createSignal, createRoot, onCleanup, type Accessor } from 'solid-js';
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
function createSnapshotWriter() {
    type Scheduler={postTask:(callback:()=>void,options:{priority:'background';signal:AbortSignal})=>Promise<void>};
    const scheduler=(window as Window&{scheduler?:Scheduler}).scheduler;
    let pending:{key:string;rows:ChatSummary[];cursor?:Cursor}|undefined,timer=0,frame=0,last=performance.now(),disposed=false,task:AbortController|undefined,saved:ChatSummary[]|undefined;
    const flush=()=>{
        clearTimeout(timer);cancelAnimationFrame(frame);task?.abort();task=undefined;timer=frame=0;
        if(disposed||!pending)return;
        const current=pending;pending=undefined;saved=current.rows;last=performance.now();
        void viewCache.write(current.key,boundedSnapshot(current.rows),current.cursor).catch(()=>{if(saved===current.rows)saved=undefined;});
    };
    const hidden=()=>{if(document.hidden)flush();};
    window.addEventListener('pagehide',flush);document.addEventListener('visibilitychange',hidden);
    onCleanup(()=>{disposed=true;pending=undefined;clearTimeout(timer);cancelAnimationFrame(frame);task?.abort();window.removeEventListener('pagehide',flush);document.removeEventListener('visibilitychange',hidden);});
    return (key:string,rows:ChatSummary[],cursor?:Cursor)=>{
        if(saved===rows&&!pending)return;
        pending={key,rows,cursor};
        if(timer||frame||task)return;
        timer=window.setTimeout(()=>{
            timer=0;
            if(document.hidden){flush();return;}
            if(scheduler){task=new AbortController();void scheduler.postTask(()=>{task=undefined;flush();},{priority:'background',signal:task.signal}).catch(()=>{});return;}
            frame=requestAnimationFrame(()=>{frame=0;timer=window.setTimeout(flush,0);});
        },Math.max(0,500-(performance.now()-last)));
    };
}
function createChatView(live: LiveClient, filter: Accessor<ChatFilter | undefined>) {
    type Page={after?:string;view:ReturnType<LiveClient['subscribe']>};
    const [pages,setPages]=createSignal<Page[]>([]),[delayed,setDelayed]=createSignal(false);
    const [ready,setReady]=createSignal(false);
    let complete:ChatSummary[]=[],cachedCursor:Cursor|undefined;
    const [completeKnown,setCompleteKnown]=createSignal(false);
    const persist=createSnapshotWriter();
    createEffect(()=>{
        const value=filter();
        complete=[];setCompleteKnown(false);cachedCursor=undefined;setReady(false);setPages([]);setDelayed(false);
        if(!value){setReady(true);return;}
        let active=true;
        const timer=setTimeout(()=>setDelayed(true),150);
        void viewCache.read(JSON.stringify(['chats-snapshot',1,value])).then(cached=>{
            if(!active)return;
            if(cached?.data.snapshot_version===1&&cached.data.chats){complete=cached.data.chats;setCompleteKnown(cached.data.snapshot_complete===true&&complete.every(chat=>typeof chat.known_sender==='boolean'));cachedCursor=completeKnown()?cached.cursor:undefined;}
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
    const rows=createMemo<ChatSummary[]>((previous)=>{
        if(!ready())return [];
        const unique=new Map<string,ChatSummary>();
        for(const page of pages())for(const chat of page.view.data().chats??[])if(chat.chat?.id&&(filter()==='deleted'||!chat.chat.data.is_archived)&&!unique.has(chat.chat.id)){const pinned=chat.chat.data.pinned_index;unique.set(chat.chat.id,chat.pinned_index==null&&typeof pinned==='number'?{...chat,pinned_index:pinned}:chat);}
        const current=pages(),last=current.at(-1)?.view.data();
        const cursor=current[0]?.view.cursor();
        const pending=current.some(page=>{const next=page.view.cursor();return !cursor||!next||next.epoch!==cursor.epoch||next.sequence!==cursor.sequence;})||current.some((page,index)=>!page.view.data().chats||page.view.data().next!==current[index+1]?.after)||Boolean(last?.next);
        if(pending)for(const chat of complete)if(!unique.has(chat.chat.id))unique.set(chat.chat.id,chat);
        const ordered=[...unique.values()].sort((a,b)=>(Date.parse(b.last_activity_at)||0)-(Date.parse(a.last_activity_at)||0)||a.chat.id.localeCompare(b.chat.id));
        if(!pending){complete=ordered;setCompleteKnown(true);}
        return ordered.length===previous.length&&ordered.every((row,index)=>row===previous[index])?previous:ordered;
    },[]);
    createEffect(()=>{
        const value=filter(),current=rows(),cursor=pages()[0]?.view.cursor()??cachedCursor;
        if(value&&ready()&&completeKnown())persist(JSON.stringify(['chats-snapshot',1,value]),current,cursor);
    });
    return {rows,loading:()=>{rows();return !ready()||!completeKnown();},delayed};
}
function buildChatsStore(live: LiveClient) {
    const [filter,setFilter]=createSignal<ChatFilter>('all');
    const all=createChatView(live,()=> 'all');
    const [specials,setSpecials]=createSignal(false);
    let timer=0;
    const frame=requestAnimationFrame(()=>{timer=window.setTimeout(()=>setSpecials(true),0);});
    onCleanup(()=>{cancelAnimationFrame(frame);clearTimeout(timer);});
    const deleted=createChatView(live,()=>specials()?'deleted':undefined),spam=createChatView(live,()=>specials()?'spam':undefined);
    const buckets=createMemo(()=>{
        const result:{all:ChatSummary[];known:ChatSummary[];unknown:ChatSummary[];unread:ChatSummary[]}={all:[],known:[],unknown:[],unread:[]};
        for(const chat of all.rows()){
            if(chat.chat.data.deleted_at)continue;
            result.all.push(chat);
            if(typeof chat.known_sender==='boolean')result[chat.known_sender?'known':'unknown'].push(chat);
            if(chat.unread_count>0)result.unread.push(chat);
        }
        return result;
    });
    const visibleAll=()=>buckets().all;
    const rows=()=>{const value=filter();return value==='deleted'?deleted.rows():value==='spam'?spam.rows():buckets()[value];};
    const unread=createMemo(()=>buckets().unread.length);
    const active=()=>filter()==='deleted'?deleted:filter()==='spam'?spam:all;
    return {all:visibleAll,rows,filter,setFilter,unread,loading:()=>active().loading(),delayed:()=>active().delayed()};
}
const stores=new WeakMap<LiveClient,{store:ReturnType<typeof buildChatsStore>;dispose:()=>void;users:number}>();
export function createChatsStore(live:LiveClient){
    let entry=stores.get(live);
    if(!entry){let dispose=()=>{};const store=createRoot(release=>{dispose=release;return buildChatsStore(live);});entry={store,dispose,users:0};stores.set(live,entry);}
    const shared=entry;shared.users++;
    onCleanup(()=>{shared.users--;if(!shared.users){shared.dispose();stores.delete(live);}});
    return shared.store;
}
export type ChatsStore=ReturnType<typeof createChatsStore>;
