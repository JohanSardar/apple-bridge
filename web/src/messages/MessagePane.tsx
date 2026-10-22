import {batch,createEffect,createRoot,createSignal,onCleanup,onMount,untrack} from 'solid-js';
import type {LiveClient} from '../data/live';
import type {Cursor,DeviceSession,ThreadItem} from '../data/types';
import {router} from '../app/router';
import {Composer} from './compose/Composer';
import type {createMessageCommands} from './actions/commands';
import Transcript from './transcript/Transcript';
import {MessageActions} from './actions/MessageActions';
import type {MessageActionsController} from './actions/reactionActions';
export default function MessagePane(props:{chatID:string;messageID?:string;title:(value:string)=>void;actions:MessageActionsController;live:LiveClient;commands:ReturnType<typeof createMessageCommands>;session:DeviceSession}){
    const items=new Map<string,ThreadItem>(),commands=props.commands;
    const chat=props.live.subscribe('record',{collection:'messages.chat',id:props.chatID},{cache:false,resetOnReconnect:true});
    const [baseline,setBaseline]=createSignal<Cursor>(),[commandID,setCommandID]=createSignal(untrack(()=>commands.statuses(props.chatID).at(-1)?.id??''));
    const reply=()=>{const value=props.actions.replying();return value?.item.data.chat_id===props.chatID?value:undefined;};
    const available=()=>{const observed=baseline(),cursor=chat.cursor();return Boolean(observed&&cursor&&observed.epoch===cursor.epoch&&observed.subscription_revision===cursor.subscription_revision&&commands.capabilities.allowed(reply()?'messages.reply':'messages.send'));};
    const service=()=>String(chat.data().records?.find(record=>record.id===props.chatID)?.data.service??'');
    const status=()=>commands.statuses().find(command=>command.id===commandID());
    const quote=()=>{const value=reply();if(!value)return;const part=Array.isArray(value.item.data.parts)?value.item.data.parts.find(part=>part.index===value.partIndex):undefined;return {key:value.item.id+'/'+value.partIndex,text:part?.kind==='text'?String(part.text??''):'Attachment'};};
    const notice=()=>{const values=[commands.notice(props.chatID),props.actions.notice(props.chatID)].filter(value=>value!==undefined);return values.length?{count:values.reduce((sum,value)=>sum+value.count,0),command_ids:[...new Set(values.flatMap(value=>value.command_ids))],text:values.map(value=>value.text).join(' · ')}:undefined;};
    const check=(id:string)=>{if(commands.statuses().some(command=>command.id===id))commands.check(id);else props.actions.checkID(id);};
    const send=(event:Event,content:{text:string})=>{if(!available())throw new Error('Sending unavailable');const target=reply(),id=target?commands.reply(event,target.item,target.partIndex,content.text):commands.send(event,{chat_id:props.chatID,text:content.text});setCommandID(id);props.actions.cancelReply();return id;};
    createEffect(()=>{if(props.live.status()!=='connected')setBaseline();});
    onCleanup(()=>{chat.dispose();if(reply())props.actions.cancelReply();});

    let host!:HTMLDivElement,press:{event:PointerEvent;item:ThreadItem;x:number;y:number}|undefined,timer:ReturnType<typeof setTimeout>|undefined,suppressClickUntil=0;
    let pendingShortcut:{event:KeyboardEvent;id:string;mode:'react'|'reply';revealed?:boolean}|undefined,cancelLatest:(()=>void)|undefined,shortcutTimer:ReturnType<typeof setTimeout>|undefined,shortcutFrame=0;
    const anchorFor=(id:string)=>{const cell=host.querySelector<HTMLElement>('[data-message="'+CSS.escape(id)+'"]');if(cell?.closest('[inert]'))return;return cell?.hasAttribute('data-media-origin')?cell:cell?.querySelector<HTMLElement>('.bubble');};
    const revealShortcut=()=>{
        const pending=pendingShortcut;if(!pending)return;
        const anchor=anchorFor(pending.id);if(anchor){anchor.scrollIntoView({block:'nearest',inline:'nearest'});return;}
        if(pending.revealed)return;pending.revealed=true;
        if(props.messageID!==pending.id)router.navigate('/messages/'+encodeURIComponent(props.chatID)+'/'+encodeURIComponent(pending.id),true);
        else{const viewport=host.querySelector<HTMLElement>('[role="listbox"][aria-label="Messages transcript"]');if(viewport)viewport.scrollTop=viewport.scrollHeight;}
    };
    const cancelShortcut=()=>{cancelLatest?.();cancelLatest=undefined;clearTimeout(shortcutTimer);cancelAnimationFrame(shortcutFrame);pendingShortcut=undefined;};
    const openShortcut=()=>{
        if(!pendingShortcut)return;
        cancelAnimationFrame(shortcutFrame);
        shortcutFrame=requestAnimationFrame(()=>{
            const pending=pendingShortcut;if(!pending)return;
            if(!(pending.mode==='reply'?props.actions.canReplyOperation():props.actions.canReact())||host.closest('[inert]')){cancelShortcut();return;}
            const item=items.get(pending.id),anchor=anchorFor(pending.id);
            if(!item||!anchor){revealShortcut();return;}
            anchor.scrollIntoView({block:'nearest',inline:'nearest'});pendingShortcut=undefined;clearTimeout(shortcutTimer);batch(()=>{props.actions.open(pending.event,item,anchor);if(pending.mode==='reply')props.actions.beginReply(pending.event);});
        });
    };
    const latest=(event:KeyboardEvent,mode:'react'|'reply')=>{
        event.preventDefault();cancelShortcut();
        try{
            const view=props.live.subscribe('thread',{chat_id:props.chatID,anchor:'latest',limit:1},{cache:false});
            cancelLatest=createRoot(dispose=>{
                createEffect(()=>{
                    const captured=view.data().items,cursor=view.cursor();if(!cursor||!captured)return;
                    const item=captured.at(-1);dispose();view.dispose();cancelLatest=undefined;
                    if(!item||!host.isConnected||host.closest('[inert]'))return;
                    pendingShortcut={event,id:item.id,mode};
                    revealShortcut();
                    openShortcut();
                });
                return()=>{dispose();view.dispose();};
            });
            shortcutTimer=setTimeout(cancelShortcut,10000);
        }catch{cancelShortcut();}
    };
    const cancel=()=>{clearTimeout(timer);press=undefined;};
    const start=(event:PointerEvent)=>{
        cancel();if(event.pointerType!=='touch'||!event.isTrusted)return;
        const row=event.target instanceof Element?event.target.closest<HTMLElement>('[data-message]'):undefined,item=row&&items.get(row.dataset.message??'');
        if(!item)return;
        press={event,item,x:event.clientX,y:event.clientY};timer=setTimeout(()=>{if(press){props.actions.open(press.event,press.item);if(props.actions.target()?.item.id===press.item.id)suppressClickUntil=performance.now()+750;}press=undefined;},500);
    };
    onMount(()=>{
        const trailingClick=(event:MouseEvent)=>{if(performance.now()<suppressClickUntil){suppressClickUntil=0;event.preventDefault();event.stopImmediatePropagation();}};
        host.addEventListener('click',trailingClick,true);onCleanup(()=>host.removeEventListener('click',trailingClick,true));
        const observer=new MutationObserver(openShortcut);observer.observe(host,{childList:true,subtree:true});onCleanup(()=>observer.disconnect());
        const keyboard=(event:KeyboardEvent)=>{const key=event.key.toLowerCase(),mode=key==='r'?'reply':'react';if(!(event.metaKey||event.ctrlKey)||event.altKey||event.shiftKey||!['t','r'].includes(key)||event.repeat||event.isComposing||!items.size||!(mode==='reply'?props.actions.canReplyOperation():props.actions.canReact())||host.closest('[inert]')||document.querySelector('dialog[open]'))return;latest(event,mode);};
        window.addEventListener('keydown',keyboard);onCleanup(()=>window.removeEventListener('keydown',keyboard));
    });
    onCleanup(()=>{cancel();cancelShortcut();props.actions.close();});
    return <div ref={host} class="message-pane" style={{display:'contents'}} onPointerDown={start} onPointerUp={cancel} onPointerCancel={cancel} onPointerMove={event=>{if(press&&Math.hypot(event.clientX-press.x,event.clientY-press.y)>10)cancel();}}>
        <Transcript chatID={props.chatID} messageID={props.messageID} title={props.title} onObserved={(captured,cursor)=>{if(!cursor)return;items.clear();for(const item of captured)items.set(item.id,item);props.actions.observe(captured,cursor,props.chatID);commands.observe(captured,cursor,props.chatID);setBaseline(cursor);openShortcut();}} pending={()=>commands.pending().filter(item=>item.data.chat_id===props.chatID)} projectCaptured={commands.projectCaptured} deliveryNotice={notice} onCheckCommand={check} onCopyCommand={commands.copy} composer={<Composer session={props.session} draftID={props.chatID} service={service()} available={available()} reply={quote()} onCancelReply={props.actions.cancelReply} command={status()} canCheck={props.live.status()==='connected'} onSend={send} onCheck={()=>check(commandID())} onCopy={event=>{void commands.copy(event,commandID());}}/>} onMessageAction={props.actions.open}/>
        <MessageActions actions={props.actions}/>
    </div>;
}
