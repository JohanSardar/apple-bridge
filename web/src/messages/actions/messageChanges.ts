import {createEffect,createMemo,createSignal,onCleanup,untrack} from 'solid-js';
import type {LiveClient} from '../../data/live';
import type {BridgeRecord,CommandState,Cursor,ThreadItem} from '../../data/types';
import {withInput} from '../../data/input';
import type {ActionTarget} from './reactionActions';
type Change={id:string;kind:'edit'|'unsend';target:ActionTarget;text?:string;complete?:boolean};
type Editor={target:ActionTarget;text:string;commandID?:string};
const observed=(state:CommandState|undefined)=>state&&['observed','delivered'].includes(state.status);
const uncertain=(state:CommandState|undefined)=>state&&(state.transport_lost||['indeterminate','delivery_failed'].includes(state.status)||['indeterminate','delivery_failed'].includes(state.disposition??''));
const failed=(state:CommandState|undefined)=>state&&['failed','rejected','expired'].includes(state.status)&&!uncertain(state);
const compatible=(first:Cursor,second:Cursor)=>first.epoch===second.epoch&&first.subscription_revision===second.subscription_revision;
export function createMessageChanges(live:LiveClient,allowed:(operation:string)=>boolean,current:()=>ActionTarget|undefined,close:()=>void){
    const [revision,setRevision]=createSignal(0),[editor,setEditor]=createSignal<Editor>(),[clock,setClock]=createSignal(Date.now()),[error,setError]=createSignal('');
    const messages=new Map<string,{item:ThreadItem;cursor:Cursor}>(),chats=new Map<string,{record:BridgeRecord;cursor:Cursor}>(),changes=new Map<string,Change>();
    let finishFrame=0;
    const complete=(change:Change)=>{if(change.complete)return;change.complete=true;if(editor()?.commandID===change.id){cancelAnimationFrame(finishFrame);finishFrame=requestAnimationFrame(()=>{if(editor()?.commandID===change.id){setEditor();setError('');}});}};
    const changed=()=>setRevision(value=>value+1),state=(id:string)=>live.pending().find(command=>command.id===id);
    const timer=setInterval(()=>setClock(Date.now()),1000);
    createEffect(()=>{if(live.status()!=='connected'){messages.clear();chats.clear();changed();}});
    const parts=(item:ThreadItem):{index:number;kind?:string;text?:string}[]=>Array.isArray(item.data.parts)&&item.data.parts.length?item.data.parts.filter(part=>part&&typeof part==='object'&&Number.isSafeInteger(part.index)&&part.index>=0):typeof item.data.text==='string'?[{index:0,kind:'text',text:item.data.text}]:[];
    const partText=(target:ActionTarget)=>{const part=parts(target.item).find(part=>part.index===target.partIndex);return part?.kind==='text'?String(part.text??(target.partIndex===0?target.item.data.text??'':'')):undefined;};
    const locked=(target:ActionTarget)=>[...changes.values()].some(change=>!change.complete&&change.target.item.id===target.item.id&&change.target.partIndex===target.partIndex&&!failed(state(change.id)));
    const eligible=(target:ActionTarget|undefined,kind:'edit'|'unsend')=>{
        revision();clock();const now=Date.now();if(!target||live.status()!=='connected'||!allowed('messages.'+kind))return false;
        const capture=messages.get(target.item.id),chat=chats.get(String(target.item.data.chat_id??''));
        if(!capture||!chat||!compatible(capture.cursor,chat.cursor)||chat.record.data.service!=='iMessage')return false;
        const item=capture.item,age=(now-Date.parse(item.data.sent_at))/1000;
        if(item.id.startsWith('command:')||item.collection!=='messages.message'||!Number.isSafeInteger(item.version)||item.version<1||item.data.is_from_me!==true||item.data.deleted_at||item.data.retracted_at||!Number.isFinite(age)||age<0||age>=(kind==='edit'?900:120)||!Number.isSafeInteger(target.partIndex)||target.partIndex<0||!parts(item).some(part=>part.index===target.partIndex)||Array.isArray(item.data.retracted_parts)&&item.data.retracted_parts.includes(target.partIndex))return false;
        if(kind==='edit'&&(partText({...target,item})===undefined||!Array.isArray(item.data.edit_history)&&item.data.edit_history!=null||Array.isArray(item.data.edit_history)&&item.data.edit_history.length>=5))return false;
        return !locked(target);
    };
    const canEdit=()=>!editor()&&!current()?.anchor.closest('dialog[aria-label="Replies"]')&&eligible(current(),'edit'),canUnsend=()=>!dissolve()&&!current()?.anchor.closest('dialog[aria-label="Replies"]')&&eligible(current(),'unsend');
    const cancelEdit=()=>{const value=editor();if(value?.commandID&&!failed(state(value.commandID)))return;setEditor();setError('');queueMicrotask(()=>{const anchor=value?.target.anchor;if(anchor?.isConnected){const previous=anchor.getAttribute('tabindex');anchor.tabIndex=-1;anchor.focus({preventScroll:true});if(previous===null)anchor.removeAttribute('tabindex');else anchor.setAttribute('tabindex',previous);}});};
    const beginEdit=(event:Event)=>{const target=current();if(!event.isTrusted||!target||!canEdit()||target.anchor.closest('dialog[aria-label="Replies"]'))return;setEditor({target:{...target,item:messages.get(target.item.id)!.item},text:partText(target)??''});setError('');close();};
    const editState=()=>{const id=editor()?.commandID;return id?state(id):undefined;};
    const editBusy=()=>Boolean(editor()?.commandID&&!failed(editState()));
    const editProblem=()=>{revision();const value=editor();return error()||(editState()?.check_error?"Couldn't check this edit. Try again.":failed(editState())?"Couldn't edit this message.":!observed(editState())&&uncertain(editState())?'Not sure if the message changed.':value&&!value.commandID&&messages.get(value.target.item.id)?.item.version!==value.target.item.version?'Message changed. Cancel and edit it again.':value&&!value.commandID&&!eligible(value.target,'edit')?'This message can no longer be edited.':'');};
    const editLength=createMemo(()=>new TextEncoder().encode(editor()?.text??'').length);
    const validEdit=()=>{const value=editor();revision();if(!value||editBusy()||!eligible(value.target,'edit')||messages.get(value.target.item.id)?.item.version!==value.target.item.version)return false;return value.text.trim().length>0&&editLength()<=65536&&value.text!==partText(value.target);};
    const copyEdit=(event:Event)=>{if(!event.isTrusted)return;const text=editor()?.text;if(text===undefined)return;try{withInput(event,()=>{void navigator.clipboard.writeText(text).catch(()=>setError("Couldn't copy this edit."));});}catch{setError("Couldn't copy this edit.");}};
    const dispatch=(event:Event,target:ActionTarget,kind:'edit'|'unsend',text?:string)=>withInput(event,()=>{
        const id=live.command('messages.'+kind,{id:target.item.id,part_index:target.partIndex,...(kind==='edit'?{text}: {})},undefined,target.item.version);
        changes.set(id,{id,kind,target,text});changed();return id;
    });
    const saveEdit=(event:Event)=>{const value=editor();if(!event.isTrusted||!value||!validEdit())return;try{const id=dispatch(event,value.target,'edit',value.text);setEditor({...value,commandID:id});setError('');}catch{setError("Can't edit right now. Check your connection.");}};
    const [dissolve,setDissolve]=createSignal<{target:ActionTarget;id:string}>();
    createEffect(()=>{revision();const value=dissolve();if(!value)return;const command=state(value.id);if(changes.get(value.id)?.complete||failed(command)||uncertain(command)||observed(command)&&!command?.observed_record_ids?.length)setDissolve();});
    const unsend=(event:Event)=>{const value=current();if(!event.isTrusted||!value||!canUnsend()||value.anchor.closest('dialog[aria-label="Replies"]'))return;try{const target={...value,item:messages.get(value.item.id)!.item},id=dispatch(event,target,'unsend');setDissolve({target,id});close();}catch{setError("Can't undo send right now. Check your connection.");}};
    const check=(id:string)=>{if(!changes.has(id))return false;try{live.checkCommand(id);}catch{setError("Couldn't check this change. Try again.");}return true;};
    const notice=(chatID:string)=>{
        revision();const values=[...changes.values()].filter(change=>!change.complete&&change.target.item.data.chat_id===chatID);
        if(!values.length)return;
        const text=values.map(change=>{const command=state(change.id),noun=change.kind==='edit'?'edit':'undo send';if(command?.check_error)return "Couldn't check this change. Try again.";if(failed(command))return change.kind==='edit'?"Message wasn't edited":"Message wasn't unsent";if(observed(command))return 'Waiting for the updated message';if(uncertain(command))return 'Not sure if the message changed';return noun==='edit'?'Saving edit…':'Undoing send…';});
        return {count:values.length,command_ids:values.map(change=>change.id),text:[...new Set(text)].join(' · ')};
    };
    const observeChat=(record:BridgeRecord|undefined,cursor:Cursor|undefined)=>{if(!record||!cursor||live.status()!=='connected')return;chats.set(record.id,{record,cursor});changed();};
    const leaveChat=(chatID:string)=>{chats.delete(chatID);if(editor()?.target.item.data.chat_id===chatID){setEditor();setError('');}for(const [id,value] of messages)if(value.item.data.chat_id===chatID)messages.delete(id);for(const [id,change] of changes)if(change.complete&&change.target.item.data.chat_id===chatID)changes.delete(id);changed();};
    const reconcile=()=>{
        let completed=false;
        for(const [id,change] of changes){
            if(change.complete)continue;
            const command=state(id),capture=messages.get(change.target.item.id);
            if(observed(command)&&command?.cursor&&command.commit_seq&&capture&&compatible(command.cursor,capture.cursor)&&capture.cursor.sequence>=command.commit_seq&&command.observed_record_ids?.includes(change.target.item.id)){
                complete(change);completed=true;
            }
        }
        if(completed)changed();
    };
    createEffect(()=>{revision();live.pending();untrack(reconcile);});
    const observe=(items:readonly ThreadItem[],cursor:Cursor|undefined,chatID:string)=>{
        if(!cursor||live.status()!=='connected')return;
        const ids=new Set(items.map(item=>item.id));for(const item of items)messages.set(item.id,{item,cursor});
        for(const [id,change] of changes){
            if(change.target.item.data.chat_id!==chatID)continue;
            const command=state(id);
            if(observed(command)&&command?.cursor&&command.commit_seq&&compatible(command.cursor,cursor)&&cursor.sequence>=command.commit_seq&&command.observed_record_ids?.includes(change.target.item.id)){
                complete(change);
            }
            if((change.complete||failed(command)&&editor()?.commandID!==id)&&!ids.has(change.target.item.id))changes.delete(id);
        }
        for(const [id,value] of messages)if(value.item.data.chat_id===chatID&&!ids.has(id)&&editor()?.target.item.id!==id&&![...changes.values()].some(change=>!change.complete&&change.target.item.id===id))messages.delete(id);
        changed();
    };
    onCleanup(()=>{clearInterval(timer);cancelAnimationFrame(finishFrame);messages.clear();chats.clear();changes.clear();});
    const pendingEdit=()=>{revision();const target=current();return target?[...changes.values()].findLast(change=>!change.complete&&change.kind==='edit'&&change.target.item.id===target.item.id&&change.target.partIndex===target.partIndex):undefined;};
    const copyPendingEdit=(event:Event)=>{const change=pendingEdit();if(!event.isTrusted||!change?.text)return;try{withInput(event,()=>{void navigator.clipboard.writeText(change.text!).catch(()=>setError("Couldn't copy this edit."));});}catch{setError("Couldn't copy this edit.");}};
    return {canEdit,canUnsend,canOperation:(kind:'edit'|'unsend')=>live.status()==='connected'&&allowed('messages.'+kind),beginEdit,editor,setEditText:(text:string)=>setEditor(value=>value?{...value,text}:value),cancelEdit,copyEdit,pendingEdit,copyPendingEdit,saveEdit,validEdit,editLength,editBusy,editState,editProblem,unsend,dissolve,clearDissolve:()=>setDissolve(),check,notice,observe,observeChat,leaveChat};
}
