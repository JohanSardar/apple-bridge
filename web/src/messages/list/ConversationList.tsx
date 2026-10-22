import { strings as s } from '../../app/strings';
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { Icon } from '../../design/Icon';
import { router } from '../../app/router';
import { registerShortcut } from '../../app/keyboard';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import { createReorderMotion } from './reorder';
import { createUnreadPulses } from './unread';
import { createChatsStore, chatFilters } from '../store/chats';
import { createHandlesCache } from '../store/handles';
import { fields, text } from '../store/types';
import { ConversationRow } from './ConversationRow';
import { PinnedGrid } from './PinnedGrid';
import { SearchField } from './SearchField';
import { FiltersMenu } from './FiltersMenu';
import { conversationTitle } from '../format/list';
import './list.css';
export function ConversationList(props:{live:LiveClient;onFullSearch?:(query:string)=>void}) {
    const store=createChatsStore(props.live), cache=createHandlesCache(props.live);
    const [focused,setFocused]=createSignal(document.hasFocus()&&document.visibilityState==='visible');
    const [query,setQuery]=createSignal(router.route().chat==='search'?new URLSearchParams(location.search).get('q')??'':'');
    let search!:HTMLInputElement,root!:HTMLDivElement,virtual:VirtualListHandle|undefined;
    const phoneMedia=matchMedia('(max-width:699px)'),[phone,setPhone]=createSignal(phoneMedia.matches);
    const handles=createMemo(()=>cache.read());
    const participants=(chat:ChatSummary)=>Array.from(new Set([...(chat.avatar_handles??[]).map(member=>member.id),...(Array.isArray(chat.chat.data.participants)?chat.chat.data.participants.filter((id):id is string=>typeof id==='string'):[])]));
    const folded=(value:string)=>value.normalize('NFD').replace(/\p{M}/gu,'').toLocaleLowerCase();
    const visible=createMemo(()=>{const needle=folded(query().trim());return needle?store.rows().filter(chat=>folded([conversationTitle(chat,handles()),...participants(chat).map(id=>handles().get(id)?.display_name??'')].join(' ')).includes(needle)):store.rows();});
    const fullSearch=(value:string)=>props.onFullSearch?props.onFullSearch(value):router.navigate('/messages/search?q='+encodeURIComponent(value));
    onCleanup(()=>store.setFilter('all'));
    const pinnedChats=createMemo(()=>store.all().filter(chat=>chat.pinned_index!==null&&chat.pinned_index!==undefined).sort((a,b)=>a.pinned_index!-b.pinned_index!).slice(0,9));
    const pins=createMemo(()=>visible().filter(chat=>chat.pinned_index!==null&&chat.pinned_index!==undefined).sort((a,b)=>a.pinned_index!-b.pinned_index!).slice(0,9));
    const rows=createMemo(()=>visible().filter(chat=>!pins().includes(chat)));
    const selected=()=>router.route().chat;
    const activeFilter=()=>chatFilters.find(filter=>filter.value===store.filter())!;
    const open=(chat:ChatSummary)=>router.navigate('/messages/'+encodeURIComponent(chat.chat.id));
    const unreadPulses=createUnreadPulses(store.all);
    createEffect(()=>{cache.seed(store.all());cache.ensure(store.all().flatMap(chat=>{const event=fields(chat.last_message?.preview.group_event);return [...participants(chat),chat.last_message?.sender_handle_id??'',text(event.handle_id??event.target_handle_id)];}));});
    createReorderMotion(()=>root,rows);
    function cycle(step:number){const chats=[...pins(),...rows()];if(!chats.length)return;const index=chats.findIndex(chat=>chat.chat.id===selected()),next=index<0?(step>0?0:chats.length-1):(index+step+chats.length)%chats.length;open(chats[next]);const row=rows().findIndex(chat=>chat.chat.id===chats[next].chat.id);if(row>=0)virtual?.scrollToIndex(row);}
    onMount(()=>{
        const focusChanged=()=>setFocused(document.hasFocus()&&document.visibilityState==='visible');
        const mediaChanged=()=>setPhone(phoneMedia.matches);
        const keyboard=(event:KeyboardEvent)=>{
            if(phone()||event.isComposing)return;
            if(event.ctrlKey&&event.key==='Tab'){event.preventDefault();cycle(event.shiftKey?-1:1);}
        };
        const searchShortcuts=[false,true].map(alt=>registerShortcut({key:'f',alt,input:true,action:()=>search.focus()}));
        const pinShortcuts=Array.from({length:9},(_,index)=>registerShortcut({key:String(index+1),input:true,action:()=>{const pin=pinnedChats()[index];if(pin)open(pin);}}));
        window.addEventListener('keydown',keyboard,true);window.addEventListener('focus',focusChanged);window.addEventListener('blur',focusChanged);document.addEventListener('visibilitychange',focusChanged);phoneMedia.addEventListener('change',mediaChanged);
        onCleanup(()=>{pinShortcuts.forEach(dispose=>dispose());searchShortcuts.forEach(dispose=>dispose());window.removeEventListener('keydown',keyboard,true);window.removeEventListener('focus',focusChanged);window.removeEventListener('blur',focusChanged);document.removeEventListener('visibilitychange',focusChanged);phoneMedia.removeEventListener('change',mediaChanged);});
    });
    return <div ref={root} class="conversation-list" classList={{'window-inactive':!focused()}} data-filter={activeFilter().label}><header class="toolbar sidebar-toolbar"><FiltersMenu value={store.filter()} onChange={store.setFilter}/><Show when={!phone()}><div class="list-search-bar desktop-search"><SearchField value={query()} onInput={setQuery} onSearch={fullSearch} ref={element=>{search=element;}}/></div></Show><span class="phone-toolbar-spacer"/><button class="icon-button" aria-label={s.compose} onClick={()=>router.navigate('/messages/new')}><Icon name="compose"/></button></header><div class="sidebar-heading"><h1>{s.messages}</h1></div><Show when={props.live.status()==='offline'}><p class="offline-banner" role="status">{s.offline}</p></Show><Show when={pins().length}><PinnedGrid chats={pins()} handles={handles()} selected={selected()} onOpen={open}/></Show><div class="conversation-list-body"><Show when={rows().length} fallback={<Show when={store.loading()&&store.delayed()} fallback={<Show when={!store.loading()}><div class="list-empty" role="status"><h2>{query().trim()?s.noResults:activeFilter().empty}</h2></div></Show>}><div class="list-skeleton" aria-label={s.loadingConversations} aria-busy="true"><For each={[0,1,2,3,4,5,6]}>{()=> <div class="skeleton-row"><span/><span><i/><i/></span></div>}</For></div></Show>}><VirtualList items={rows()} keyFor={chat=>chat.chat.id} estimate={()=>phone()?84:68} label={s.conversations} fixedHeights selectedKey={selected()} keepTop onActive={open} ref={handle=>virtual=handle} onVisible={(start,end)=>cache.ensure(rows().slice(start,end).flatMap(chat=>(chat.avatar_handles??[]).map(handle=>handle.id)))} render={chat=><ConversationRow chat={chat} handles={handles()} selected={selected()===chat.chat.id} unreadPulse={unreadPulses().get(chat.chat.id)} onOpen={open}/>}/></Show></div><Show when={phone()}><div class="list-search-bar phone-search glass"><SearchField value={query()} onInput={setQuery} onSearch={fullSearch} ref={element=>{search=element;}}/></div></Show></div>;
}
