import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { withInput } from '../data/input';
import type { LiveClient } from '../data/live';
import type { BridgeRecord, CommandState, Cursor } from '../data/types';
import { createCapabilities } from '../reminders/capabilities';
import { object, text, type ContactsPage, type Item } from './store';
type Change = { id: string; op: string; args: Record<string, unknown>; before?: Item; watch?: ReturnType<LiveClient['subscribe']>; watchID?: string };
const failed = new Set(['failed','rejected','rejected_by_owner','expired','delivery_failed']);
const applied = (cursor: Cursor | undefined, state: CommandState) => Boolean(cursor && state.cursor && state.commit_seq && cursor.epoch === state.cursor.epoch && cursor.subscription_revision === state.cursor.subscription_revision && cursor.sequence >= state.commit_seq);
function name(data: Record<string,unknown>, order: string) {
    if(data.contact_type==='organization' && text(data.organization).trim())return text(data.organization).trim();
    const parts=(order==='family'?['family_name','given_name','middle_name']:['given_name','middle_name','family_name']).map(key=>text(data[key]).trim()).filter(Boolean);
    return parts.join(' ') || text(data.nickname).trim() || text(data.organization).trim();
}
export function createContactCommands(live: LiveClient) {
    const capabilities=createCapabilities(live,'contacts'), changes=new Map<string,Change>();
    for(const state of live.pending())if(['contacts.create','contacts.update','contacts.delete'].includes(state.op) && !failed.has(state.status))changes.set(state.id,{id:state.id,op:state.op,args:object(state.args)});
    const [revision,setRevision]=createSignal(0), [error,setError]=createSignal('');
    const states=()=>new Map(live.pending().map(state=>[state.id,state]));
    createEffect(()=>{ revision(); for(const change of changes.values()) { const state=states().get(change.id); if(state && failed.has(state.status))setError('Couldn’t save this change. Your contact was restored.'); } });
    function issue(event: Event, op: string, args: Record<string,unknown>, before?: Item) {
        setError('');
        for(const [id,change] of changes)if(failed.has(state(id)?.status??'')){change.watch?.dispose();changes.delete(id);}
        try {
            if(!capabilities.allowed(op) || before && (before.contact.version===0 || before.contact.data.writable!==true || before.contact.data._pending))throw new Error('This action is unavailable.');
            if(changes.size>=32)throw new Error('Wait for pending changes to finish.');
            const id=withInput(event,()=>live.command(op,args,undefined,before?.contact.version));
            changes.set(id,{id,op,args,before});setRevision(value=>value+1);return id;
        } catch { setError('Couldn’t save. Check your connection and try again.'); return; }
    }
    const project=(source: Accessor<ContactsPage>, sort: Accessor<string>, display: Accessor<string>, allowCreate: Accessor<boolean>) => (): ContactsPage => {
        revision();const input=source(), commands=states();
        const items=new Map((input.sections??[]).flatMap(section=>section.contacts).map(item=>[item.contact.id,item]));
        const removed=new Set<string>();
        for(const change of changes.values()) {
            const state=commands.get(change.id);if(!state || failed.has(state.status))continue;
            if(applied(input._cursor,state)) { queueMicrotask(()=>{ if(changes.get(change.id)!==change)return;change.watch?.dispose();changes.delete(change.id);setRevision(value=>value+1); });continue; }
            const id=state.observed_record_ids?.find(value=>value.startsWith('con_')) ?? change.before?.contact.id ?? (text(change.args.id)||undefined);
            if(id && change.watchID!==id) { change.watch?.dispose();change.watchID=id;change.watch=live.subscribe('record',{collection:'contacts.contact',id},{cache:false,resetOnReconnect:true}); }
            if(change.op==='contacts.delete') { if(id) { items.delete(id);removed.add(id); }continue; }
            if(change.op==='contacts.create' && !allowCreate())continue;
            const observed=change.watch?.data().records?.find(record=>record.id===id);
            const before=items.get(id??'')??change.before;
            const base=observed??before?.contact;
            const data={...(base?.data??{}),...(observed?{}:change.args),_pending:true};delete (data as Record<string,unknown>).id;
            if(!observed && 'image' in change.args) { delete (data as Record<string,unknown>).thumbnail_image;delete (data as Record<string,unknown>).derivatives; }
            const record: BridgeRecord={id:id??'pending:'+change.id,collection:'contacts.contact',source:'contacts',version:base?.version??0,data};
            const item: Item={...(before??{initials:'',avatar:null,is_me:null,conversation_ids:[]}),contact:record,display_name:name(data,display())||'No Name',sort_key:name(data,sort()).normalize('NFD').replace(/\p{M}/gu,'').toLowerCase()};
            if('image' in change.args && !observed)item.avatar=null;
            if(observed){ const photo=object(observed.data.thumbnail_image??observed.data.image),digest=text(photo.digest);item.avatar=/^[a-f0-9]{64}$/u.test(digest)?{digest,variant:'original'}:null; }
            items.set(record.id,item);
        }
        const section=(item:Item)=>/^[a-z]/u.test(item.sort_key)?item.sort_key[0].toUpperCase():'#';
        const compare=(a:string,b:string)=>a===b?0:a<b?-1:1;
        const sorted=[...items.values()].sort((a,b)=>compare(section(a),section(b))||compare(a.sort_key,b.sort_key)||compare(a.contact.id,b.contact.id));
        const sections: NonNullable<ContactsPage['sections']>=[];
        for(const item of sorted) { const letter=/^[a-z]/u.test(item.sort_key)?item.sort_key[0].toUpperCase():'#';let section=sections.find(value=>value.key===letter);if(!section) { section={key:letter,contacts:[]};sections.push(section); }section.contacts.push(item); }
        return {...input,sections:input.sections?sections:undefined,count:items.size,groups:input.groups?.map(group=>({...group,count:Math.max(0,group.count-(Array.isArray(group.group.data.member_ids)?group.group.data.member_ids.filter(id=>removed.has(text(id))).length:0))}))};
    };
    const state=(id: string)=>states().get(id);
    const pending=()=>{ revision();return [...changes.keys()].some(id=>{const value=state(id);return value && !failed.has(value.status) && !['observed','delivered','indeterminate'].includes(value.status);}); };
    const uncertain=()=>{revision();return [...changes.keys()].filter(id=>state(id)?.status==='indeterminate'||state(id)?.transport_lost);};
    onCleanup(()=>{for(const change of changes.values())change.watch?.dispose();capabilities.dispose();});
    return {issue,project,state,pending,uncertain,error,capabilities,check:(id:string)=>live.checkCommand(id)};
}
