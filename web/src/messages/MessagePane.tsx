import {batch,createEffect,createRoot,createSignal,lazy,Show,Suspense,onCleanup,onMount,untrack} from 'solid-js';
import type {LiveClient} from '../data/live';
import type {Cursor,DeviceSession,ThreadItem} from '../data/types';
import {router} from '../app/router';
import {Composer} from './compose/Composer';
import {UploadDock} from './compose/UploadDock';
import type {Uploads} from './compose/uploads';
import {prepareForward} from './compose/drafts';
import {withInput} from '../data/input';
import type {createMessageCommands} from './actions/commands';
import Transcript from './transcript/Transcript';
const MessageActions=lazy(()=>import('./actions/MessageActions').then(module=>({default:module.MessageActions})));
import type {MessageActionsController} from './actions/reactionActions';
export default function MessagePane(props:{chatID:string;messageID?:string;title:(value:string)=>void;actions:MessageActionsController;live:LiveClient;commands:ReturnType<typeof createMessageCommands>;session:DeviceSession;uploads:Uploads}){
    const [retryProblem,setRetryProblem]=createSignal('');
    const [retryTarget,setRetryTarget]=createSignal<{id:string;index?:number}>();
    const retryDraft='retry:'+props.chatID;
    const [transcriptGeneration,setTranscriptGeneration]=createSignal(1);
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
    const canRetry=(id:string,index?:number)=>props.commands.canRetry(id,index,props.commands.localFile(id,index)?.upload);
    const retry=(event:Event,id:string,index?:number)=>{if(!event.isTrusted||!canRetry(id,index))return;setRetryProblem('');const local=commands.localFile(id,index);if(local){setRetryTarget({id,index});if(!props.uploads.files(retryDraft).length)props.uploads.add(retryDraft,[local.file]);return;}try{setCommandID(commands.retry(event,id,index));}catch{setRetryProblem("Can't send right now. Check your connection.");}};
    const retrySend=(event:Event)=>{const target=retryTarget();if(!target||!event.isTrusted)return;const files=props.uploads.prepared(retryDraft);if(files.length!==1)return;try{setCommandID(commands.retry(event,target.id,target.index,files[0].upload,files[0]));}catch{setRetryProblem("Can't send right now. Check your connection.");return;}props.uploads.sent(retryDraft,files);setRetryTarget();};
    const send=(event:Event,content:{text:string})=>{if(!available())throw new Error('Sending unavailable');const files=props.uploads.files(props.chatID).length?props.uploads.prepared(props.chatID):[];if(reply()&&files.length)throw new Error('Attachments unavailable');const target=reply(),id=target?commands.reply(event,target.item,target.partIndex,content.text):commands.send(event,{chat_id:props.chatID,text:content.text,...(files.length?{files:files.map(file=>file.upload)}:{})},files);props.uploads.sent(props.chatID,files);setCommandID(id);props.actions.cancelReply();return id;};
    createEffect(()=>{
        const coherent=baseline(),id=props.messageID,deleted=props.actions.management.deletedIDs(props.chatID);if(!id||!deleted.includes(id))return;
        const neighbor=coherent?[...items.values()].findLast(item=>!deleted.includes(item.id)):undefined;
        batch(()=>{router.navigate('/messages/'+encodeURIComponent(props.chatID)+(neighbor?'/'+encodeURIComponent(neighbor.id):''),true);setBaseline();setTranscriptGeneration(value=>value+1);});
    });
    createEffect(()=>{if(props.live.status()!=='connected')setBaseline();});
    createEffect(()=>{const record=chat.data().records?.find(record=>record.id===props.chatID),cursor=chat.cursor();untrack(()=>props.actions.changes.observeChat(record,cursor));});
    onCleanup(()=>{chat.dispose();props.actions.leaveChat(props.chatID);if(reply())props.actions.cancelReply();});

    let host!:HTMLDivElement,press:{event:PointerEvent;item:ThreadItem;anchor:HTMLElement;x:number;y:number}|undefined,timer:ReturnType<typeof setTimeout>|undefined,held:{anchor:HTMLElement;until:number}|undefined;
    let pendingShortcut:{event:KeyboardEvent;id:string;mode:'react'|'reply'|'edit';revealed?:boolean}|undefined,cancelLatest:(()=>void)|undefined,shortcutTimer:ReturnType<typeof setTimeout>|undefined,shortcutFrame=0;
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
        if(!latestSent&&host.querySelector('.message-row'))latestSent=props.live.subscribe('thread',{chat_id:props.chatID,anchor:'latest',limit:100},{cache:false,resetOnReconnect:true});
        if(!pendingShortcut)return;
        cancelAnimationFrame(shortcutFrame);
        shortcutFrame=requestAnimationFrame(()=>{
            const pending=pendingShortcut;if(!pending)return;
            if(!(pending.mode==='reply'?props.actions.canReplyOperation():pending.mode==='edit'?props.actions.changes.canOperation('edit'):props.actions.canReact())||host.closest('[inert]')){cancelShortcut();return;}
            const item=items.get(pending.id),anchor=anchorFor(pending.id);
            if(!item||!anchor){revealShortcut();return;}
            anchor.scrollIntoView({block:'nearest',inline:'nearest'});pendingShortcut=undefined;clearTimeout(shortcutTimer);batch(()=>{props.actions.open(pending.event,item,anchor);if(pending.mode==='reply')props.actions.beginReply(pending.event);else if(pending.mode==='edit')props.actions.changes.beginEdit(pending.event);});
        });
    };
    const latest=(event:KeyboardEvent,mode:'react'|'reply'|'edit')=>{
        event.preventDefault();cancelShortcut();
        try{
            const view=props.live.subscribe('thread',{chat_id:props.chatID,anchor:'latest',limit:mode==='edit'?100:1},{cache:false});
            cancelLatest=createRoot(dispose=>{
                createEffect(()=>{
                    const captured=view.data().items,cursor=view.cursor();if(!cursor||!captured)return;
                    const item=mode==='edit'?captured.findLast(item=>item.data.is_from_me===true):captured.at(-1);dispose();view.dispose();cancelLatest=undefined;
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
    const pointer=(event:PointerEvent,item:ThreadItem)=>{
        if(!event.isTrusted||event.pointerType!=='touch')return;
        if(event.type!=='pointerdown'){
            if(press?.event.pointerId===event.pointerId&&(event.type==='pointerup'||event.type==='pointercancel'||Math.hypot(event.clientX-press.x,event.clientY-press.y)>10))cancel();
            return;
        }
        cancel();held=undefined;
        const origin=event.target instanceof Element?event.target:undefined,anchor=origin?.closest<HTMLElement>('[data-media-origin][data-message]')??origin?.closest<HTMLElement>('.bubble[data-part]');
        if(!anchor)return;
        press={event,item,anchor,x:event.clientX,y:event.clientY};
        timer=setTimeout(()=>{const value=press;press=undefined;if(!value?.anchor.isConnected)return;props.actions.open(value.event,value.item,value.anchor);if(props.actions.target()?.item.id===value.item.id)held={anchor:value.anchor,until:performance.now()+750};},500);
    };
    let latestSent:ReturnType<LiveClient['subscribe']>|undefined;
    onCleanup(()=>latestSent?.dispose());
    onMount(()=>{
        const trailingClick=(event:MouseEvent)=>{const value=held;if(!value)return;if(performance.now()>=value.until){held=undefined;return;}if(event.target instanceof Node&&value.anchor.contains(event.target)){held=undefined;event.preventDefault();event.stopImmediatePropagation();}};
        const finishPointer=(event:PointerEvent)=>{if(press?.event.pointerId===event.pointerId)cancel();};
        const movePointer=(event:PointerEvent)=>{if(press?.event.pointerId===event.pointerId&&Math.hypot(event.clientX-press.x,event.clientY-press.y)>10)cancel();};
        window.addEventListener('click',trailingClick,true);window.addEventListener('pointerup',finishPointer);window.addEventListener('pointercancel',finishPointer);window.addEventListener('pointermove',movePointer);
        onCleanup(()=>{window.removeEventListener('click',trailingClick,true);window.removeEventListener('pointerup',finishPointer);window.removeEventListener('pointercancel',finishPointer);window.removeEventListener('pointermove',movePointer);held=undefined;});
        const observer=new MutationObserver(openShortcut);observer.observe(host,{childList:true,subtree:true});onCleanup(()=>observer.disconnect());
        const keyboard=(event:KeyboardEvent)=>{
            const key=event.key.toLowerCase();
            if(!(event.metaKey||event.ctrlKey)||event.altKey||event.shiftKey||!['t','r','e','z'].includes(key)||event.repeat||event.isComposing||host.closest('[inert]')||document.querySelector('dialog[open]')||props.actions.changes.editor())return;
            if(key==='z'){
                if(event.target instanceof Element&&event.target.closest('input,textarea,[contenteditable=true]')||!props.actions.changes.canOperation('unsend')||!latestSent?.cursor())return;
                const item=latestSent.data().items?.findLast(item=>item.data.is_from_me===true),anchor=item&&anchorFor(item.id);
                if(!item||!anchor)return;
                batch(()=>{props.actions.open(event,item,anchor);if(props.actions.changes.canUnsend()){event.preventDefault();props.actions.changes.unsend(event);}else props.actions.close();});return;
            }
            const mode=key==='r'?'reply':key==='e'?'edit':'react';
            if(!(mode==='reply'?props.actions.canReplyOperation():mode==='edit'?props.actions.changes.canOperation('edit'):props.actions.canReact()))return;
            latest(event,mode);
        };
        window.addEventListener('keydown',keyboard);onCleanup(()=>window.removeEventListener('keydown',keyboard));
    });
    onCleanup(()=>{cancel();cancelShortcut();props.actions.close();});
    return <div ref={host} onDragOver={event=>{if(event.dataTransfer?.types.includes("Files"))event.preventDefault();}} onDrop={event=>{if(event.defaultPrevented||!event.dataTransfer?.files.length)return;event.preventDefault();if(!reply())props.uploads.add(props.chatID,Array.from(event.dataTransfer.files));}} class="message-pane">
        <Show when={transcriptGeneration()} keyed>{_generation=> <Transcript chatID={props.chatID} messageID={props.messageID} title={props.title} replyingTo={()=>reply()?.item.id} onObserved={(captured,cursor)=>{if(!cursor)return;items.clear();for(const item of captured)items.set(item.id,item);props.actions.observe(captured,cursor,props.chatID);commands.observe(captured,cursor,props.chatID);setBaseline(cursor);openShortcut();}} pending={()=>commands.pending().filter(item=>item.data.chat_id===props.chatID)} projectCaptured={commands.projectCaptured} deliveryNotice={notice} onCheckCommand={check} onCopyCommand={commands.copy} onRetryCommand={retry} canRetryCommand={canRetry} composer={<><Show when={retryProblem()}><p class="composer-error" role="alert">{retryProblem()}</p></Show><Show when={retryTarget()}><UploadDock uploads={props.uploads} draftID={retryDraft} locked><div class="composer-status">Prepare this attachment, then try again.<button class="button" aria-label="Send attachment again" disabled={!props.uploads.ready(retryDraft)} onClick={retrySend}>Try Again</button><button class="button subtle" onClick={()=>{for(const entry of props.uploads.files(retryDraft))props.uploads.remove(retryDraft,entry.key);setRetryTarget();}}>Cancel</button></div></UploadDock></Show><UploadDock uploads={props.uploads} draftID={props.chatID} disabled={Boolean(reply())}><Composer hasAttachments={props.uploads.files(props.chatID).length>0} disabled={props.uploads.files(props.chatID).length>0&&(!props.uploads.ready(props.chatID)||Boolean(reply()))} session={props.session} draftID={props.chatID} service={service()} available={available()} reply={quote()} onCancelReply={props.actions.cancelReply} command={status()} canCheck={props.live.status()==='connected'} onSend={send} onCheck={()=>check(commandID())} onCopy={commands.canCopy(commandID())?event=>{void commands.copy(event,commandID());}:undefined}/></UploadDock></>} onMessageAction={props.actions.open} onMessagePointer={pointer}/>}</Show>
        <Show when={props.actions.target()||props.actions.management.confirmation()||props.actions.changes.editor()||props.actions.changes.dissolve()||props.actions.flight()}><Suspense><MessageActions actions={props.actions} onForward={(event,text,attachment)=>withInput(event,()=>{const draft=prepareForward(props.session,text);if(attachment)void props.uploads.importAttachment(draft,attachment);props.actions.close();router.navigate('/messages/new');})}/></Suspense></Show>
    </div>;
}
