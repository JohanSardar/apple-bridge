import {createEffect,createRoot,onCleanup,onMount} from 'solid-js';
import type {LiveClient} from '../data/live';
import type {ThreadItem} from '../data/types';
import {router} from '../app/router';
import Transcript from './transcript/Transcript';
import {MessageActions} from './actions/MessageActions';
import type {MessageActionsController} from './actions/reactionActions';
export default function MessagePane(props:{chatID:string;messageID?:string;title:(value:string)=>void;actions:MessageActionsController;live:LiveClient}){
    const items=new Map<string,ThreadItem>();
    let host!:HTMLDivElement,press:{event:PointerEvent;item:ThreadItem;x:number;y:number}|undefined,timer:ReturnType<typeof setTimeout>|undefined,suppressClickUntil=0;
    let pendingShortcut:{event:KeyboardEvent;id:string}|undefined,cancelLatest:(()=>void)|undefined,shortcutTimer:ReturnType<typeof setTimeout>|undefined,shortcutFrame=0;
    const cancelShortcut=()=>{cancelLatest?.();cancelLatest=undefined;clearTimeout(shortcutTimer);cancelAnimationFrame(shortcutFrame);pendingShortcut=undefined;};
    const openShortcut=()=>{
        if(!pendingShortcut)return;
        cancelAnimationFrame(shortcutFrame);
        shortcutFrame=requestAnimationFrame(()=>{
            const pending=pendingShortcut;if(!pending)return;
            if(!props.actions.canReact()||host.closest('[inert]')){cancelShortcut();return;}
            const item=items.get(pending.id),anchor=host.querySelector<HTMLElement>('.message-row[data-message="'+CSS.escape(pending.id)+'"] .bubble');
            if(!item||!anchor)return;
            anchor.scrollIntoView({block:'nearest',inline:'nearest'});pendingShortcut=undefined;clearTimeout(shortcutTimer);props.actions.open(pending.event,item,anchor);
        });
    };
    const latest=(event:KeyboardEvent)=>{
        event.preventDefault();cancelShortcut();
        try{
            const view=props.live.subscribe('thread',{chat_id:props.chatID,anchor:'latest',limit:1},{cache:false});
            cancelLatest=createRoot(dispose=>{
                createEffect(()=>{
                    const captured=view.data().items,cursor=view.cursor();if(!cursor||!captured)return;
                    const item=captured.at(-1);dispose();view.dispose();cancelLatest=undefined;
                    if(!item||!host.isConnected||host.closest('[inert]'))return;
                    pendingShortcut={event,id:item.id};
                    if(!host.querySelector('.message-row[data-message="'+CSS.escape(item.id)+'"] .bubble'))router.navigate('/messages/'+encodeURIComponent(props.chatID)+'/'+encodeURIComponent(item.id),true);
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
        const keyboard=(event:KeyboardEvent)=>{if(!(event.metaKey||event.ctrlKey)||event.altKey||event.shiftKey||event.key.toLowerCase()!=='t'||event.repeat||event.isComposing||!items.size||!props.actions.canReact()||host.closest('[inert]')||document.querySelector('dialog[open]'))return;latest(event);};
        window.addEventListener('keydown',keyboard);onCleanup(()=>window.removeEventListener('keydown',keyboard));
    });
    onCleanup(()=>{cancel();cancelShortcut();props.actions.close();});
    return <div ref={host} class="message-pane" style={{display:'contents'}} onPointerDown={start} onPointerUp={cancel} onPointerCancel={cancel} onPointerMove={event=>{if(press&&Math.hypot(event.clientX-press.x,event.clientY-press.y)>10)cancel();}}>
        <Transcript chatID={props.chatID} messageID={props.messageID} title={props.title} onObserved={(captured,cursor)=>{if(!cursor)return;items.clear();for(const item of captured)items.set(item.id,item);props.actions.observe(captured,cursor,props.chatID);openShortcut();}} deliveryNotice={()=>props.actions.notice(props.chatID)} onCheckCommand={props.actions.checkID} onMessageAction={props.actions.open}/>
        <MessageActions actions={props.actions}/>
    </div>;
}
