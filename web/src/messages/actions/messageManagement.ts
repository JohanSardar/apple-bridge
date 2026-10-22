import {createEffect,createSignal,onCleanup,untrack} from 'solid-js';
import type {LiveClient} from '../../data/live';
import type {Cursor,ThreadItem,CommandState} from '../../data/types';
import {withInput} from '../../data/input';
import type {ActionTarget} from './reactionActions';
type Deletion={id:string;item:ThreadItem};
const observed=(command:CommandState|undefined)=>command&&['observed','delivered'].includes(command.status);
const uncertain=(command:CommandState|undefined)=>command&&(command.transport_lost||['indeterminate','delivery_failed'].includes(command.status)||['indeterminate','delivery_failed'].includes(command.disposition??''));
const failed=(command:CommandState|undefined)=>command&&['failed','rejected','expired'].includes(command.status)&&!uncertain(command);
const compatible=(a:Cursor,b:Cursor)=>a.epoch===b.epoch&&a.subscription_revision===b.subscription_revision;
export function createMessageManagement(live:LiveClient,allowed:(operation:string)=>boolean,current:()=>ActionTarget|undefined,close:()=>void){
    const [revision,setRevision]=createSignal(0),[confirmation,setConfirmation]=createSignal<ActionTarget>(),[error,setError]=createSignal('');
    const windows=new Map<string,{cursor:Cursor;items:Map<string,ThreadItem>}>(),deletions=new Map<string,Deletion>();
    const changed=()=>setRevision(value=>value+1),state=(id:string)=>live.pending().find(command=>command.id===id);
    const capture=(item:ThreadItem)=>{const window=windows.get(String(item.data.chat_id??''));return window?window.items.get(item.id)??item:undefined;};
    const eligible=(target:ActionTarget|undefined)=>{
        revision();if(!target||live.status()!=='connected'||!allowed('messages.delete_message'))return false;
        const item=capture(target.item);
        return Boolean(item&&!item.id.startsWith('command:')&&item.collection==='messages.message'&&Number.isSafeInteger(item.version)&&item.version>0&&!item.data.deleted_at&&![...deletions.values()].some(value=>value.item.id===item.id&&!failed(state(value.id))));
    };
    const canDelete=()=>eligible(current());
    const requestDelete=(event:Event)=>{const target=current();if(!event.isTrusted||!target||!canDelete())return;setConfirmation({...target,item:capture(target.item)!});setError('');close();};
    const cancelDelete=()=>{const target=confirmation();setConfirmation();setError('');queueMicrotask(()=>{if(target?.anchor.isConnected){const previous=target.anchor.getAttribute('tabindex');target.anchor.tabIndex=-1;target.anchor.focus({preventScroll:true});if(previous===null)target.anchor.removeAttribute('tabindex');else target.anchor.setAttribute('tabindex',previous);}});};
    const canConfirm=()=>{revision();const target=confirmation();return Boolean(target&&eligible(target)&&capture(target.item)?.version===target.item.version);};
    const problem=()=>{revision();const target=confirmation();return error()||(target&&!canConfirm()?"Message changed. Cancel and choose Delete again.":'');};
    const confirmDelete=(event:Event)=>{const target=confirmation();if(!event.isTrusted||!target||!canConfirm())return;try{withInput(event,()=>{const id=live.command('messages.delete_message',{id:target.item.id},undefined,target.item.version);for(const [previous,value] of deletions)if(value.item.id===target.item.id&&failed(state(previous)))deletions.delete(previous);deletions.set(id,{id,item:target.item});setConfirmation();setError('');changed();});}catch{setError("Can't delete right now. Check your connection.");}};
    const reconcile=()=>{let removed=false;for(const [id,value] of deletions){const command=state(id),window=windows.get(String(value.item.data.chat_id??''));if(observed(command)&&command?.cursor&&command.commit_seq&&window&&compatible(command.cursor,window.cursor)&&window.cursor.sequence>=command.commit_seq&&command.observed_record_ids?.includes(value.item.id)&&!window.items.has(value.item.id)){deletions.delete(id);removed=true;}}if(removed)changed();};
    createEffect(()=>{revision();live.pending();untrack(reconcile);});
    createEffect(()=>{if(live.status()!=='connected'){windows.clear();changed();}});
    const observe=(items:readonly ThreadItem[],cursor:Cursor|undefined,chatID:string)=>{if(!cursor||live.status()!=='connected')return;windows.set(chatID,{cursor,items:new Map(items.filter(item=>item.data.chat_id===chatID).map(item=>[item.id,item]))});reconcile();changed();};
    const check=(id:string)=>{if(!deletions.has(id))return false;try{live.checkCommand(id);}catch{setError("Couldn't check this deletion. Try again.");}return true;};
    const notice=(chatID:string)=>{revision();const values=[...deletions.values()].filter(value=>value.item.data.chat_id===chatID);if(!values.length)return;const labels=values.map(value=>{const command=state(value.id);return command?.check_error?"Couldn't check this deletion. Try again.":observed(command)?'Waiting for the deleted message to disappear':uncertain(command)?'Not sure if the message was deleted':failed(command)?"Message wasn't deleted":'Deleting message…';});return {count:values.length,command_ids:values.map(value=>value.id),text:[...new Set(labels)].join(' · ')};};
    const deletedIDs=(chatID:string)=>{revision();return [...deletions.values()].filter(value=>{const command=state(value.id);return live.status()==='connected'&&value.item.data.chat_id===chatID&&observed(command)&&command?.cursor&&command.commit_seq&&command.observed_record_ids?.includes(value.item.id);}).map(value=>value.item.id);};
    const leaveChat=(chatID:string)=>{windows.delete(chatID);if(confirmation()?.item.data.chat_id===chatID)setConfirmation();};
    onCleanup(()=>{windows.clear();deletions.clear();});
    return {canDelete,requestDelete,confirmation,cancelDelete,confirmDelete,canConfirm,problem,observe,notice,check,deletedIDs,leaveChat};
}
