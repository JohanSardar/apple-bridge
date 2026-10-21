import { createEffect, createSignal, type Accessor } from 'solid-js';
import type { ChatSummary } from '../../data/types';
export function createUnreadPulses(chats:Accessor<readonly ChatSummary[]>) {
    const [pulses,setPulses]=createSignal<ReadonlyMap<string,number>>(new Map());
    const moments=new Map<string,number>();
    let previous=new Map<string,boolean>(),initialized=false;
    createEffect(()=>{
        const rows=chats(),now=performance.now(),next=new Map(rows.map(chat=>[chat.chat.id,chat.unread_count>0]));
        if(initialized)for(const [id,unread] of next)if(unread&&!previous.get(id))moments.set(id,now);
        if(rows.length)initialized=true;
        previous=next;
        for(const [id,at] of moments)if(!next.has(id)||now-at>600)moments.delete(id);
        setPulses(new Map(moments));
    });
    return pulses;
}
