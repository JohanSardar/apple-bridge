import {createSignal,onCleanup} from 'solid-js';
import type {LiveClient} from '../../data/live';
import type {CommandState,Cursor,ThreadItem} from '../../data/types';
import {withInput} from '../../data/input';
export const classicReactions = [
    {kind:'love',label:'Love',symbol:'❤️'},{kind:'like',label:'Like',symbol:'👍'},
    {kind:'dislike',label:'Dislike',symbol:'👎'},{kind:'laugh',label:'Laugh',symbol:'😂'},
    {kind:'emphasize',label:'Emphasize',symbol:'‼️'},{kind:'question',label:'Question',symbol:'❓'},
] as const;
export type ReactionChoice={kind:string;emoji?:string;symbol:string};
export type ActionTarget={item:ThreadItem;partIndex:number;anchor:HTMLElement};
type Intent={commandID:string;messageID:string;chatID:string;partIndex:number;choice:ReactionChoice;remove:boolean};
const final=new Set(['observed','delivered','failed','rejected','expired']);
const observed=(command:CommandState)=>['observed','delivered'].includes(command.status);
const uncertain=(command:CommandState|undefined)=>command?.status==='indeterminate'||command?.status==='delivery_failed'||command?.disposition==='indeterminate'||command?.disposition==='delivery_failed';
const family=(choice:ReactionChoice)=>choice.kind==='emoji'?'emoji:'+choice.emoji:'classic';
function nativeParts(item:ThreadItem):{index:number;kind?:string;text?:string}[]{
    if(Array.isArray(item.data.parts)&&item.data.parts.length)return item.data.parts.filter(part=>part&&typeof part==='object'&&Number.isSafeInteger(part.index)&&part.index>=0);
    return typeof item.data.text==='string'?[{index:0,kind:'text',text:item.data.text}]:[];
}
export function createMessageActions(live:LiveClient,capabilities:{allowed:(operation:string)=>boolean}){
    const [target,setTarget]=createSignal<ActionTarget>(),[revision,setRevision]=createSignal(0),[error,setError]=createSignal('');
    const [flight,setFlight]=createSignal<{symbol:string;color:string;from:DOMRect;to:DOMRect;key:number}>();
    const captured=new Map<string,ThreadItem>(),intents=new Map<string,Intent>();
    let flightKey=0;
    const changed=()=>setRevision(value=>value+1);
    const current=()=>{const value=target();return value?{...value,item:captured.get(value.item.id)??value.item}:undefined;};
    const command=(intent:Intent)=>live.pending().find(value=>value.id===intent.commandID);
    const relevant=()=>{revision();const value=current();return value?[...intents.values()].filter(intent=>intent.messageID===value.item.id&&intent.partIndex===value.partIndex):[];};
    const unknown=()=>relevant().some(intent=>uncertain(command(intent)));
    const busy=()=>relevant().some(intent=>{const value=command(intent);return !value||!final.has(value.status);});
    const nativeSelected=(value:ActionTarget,choice:ReactionChoice)=>value.item.reactions.some(record=>record.data.is_from_me===true&&record.data.removed!==true&&Number(record.data.part_index??0)===value.partIndex&&record.data.kind===choice.kind&&(choice.kind!=='emoji'||record.data.emoji===choice.emoji));
    const selected=(choice:ReactionChoice)=>{
        revision();const value=current();if(!value)return false;
        const intent=intents.get(value.item.id+'/'+value.partIndex+'/'+family(choice));
        const state=intent&&command(intent);
        if(intent&&state&&(!final.has(state.status)||observed(state)))return !intent.remove&&intent.choice.kind===choice.kind&&intent.choice.emoji===choice.emoji;
        return nativeSelected(value,choice);
    };
    const close=()=>{const anchor=target()?.anchor;setTarget();setError('');queueMicrotask(()=>{if(anchor?.isConnected){const focus=anchor.closest<HTMLElement>('button,[tabindex]')??anchor;const before=focus.getAttribute('tabindex');if(before===null)focus.tabIndex=-1;focus.focus({preventScroll:true});if(before===null)focus.removeAttribute('tabindex');}});};
    const open=(event:MouseEvent|PointerEvent|KeyboardEvent,item:ThreadItem,anchorOverride?:HTMLElement)=>{
        if(!event.isTrusted||item.id.startsWith('command:')||item.collection!=='messages.message'||item.version<1||item.data.kind==='group_event'||item.data.retracted_at)return;
        const origin=event.target instanceof Element?event.target:undefined;
        const bubble=anchorOverride??origin?.closest<HTMLElement>('.bubble[data-part]')??origin?.closest('.bubble-slot')?.querySelector<HTMLElement>('.bubble[data-part]');
        const parts=nativeParts(item),index=bubble?Number(bubble.dataset.part):parts[0]?.index;
        if(index===undefined||!Number.isSafeInteger(index)||index<0||!parts.some(part=>part.index===index)||Array.isArray(item.data.retracted_parts)&&item.data.retracted_parts.includes(index))return;
        const anchor=bubble??(event.currentTarget instanceof HTMLElement?event.currentTarget:origin?.closest<HTMLElement>('[data-message]'));
        if(!anchor)return;
        if(target()?.item.id===item.id&&target()?.partIndex===index){event.preventDefault();return;}
        event.preventDefault();captured.set(item.id,item);setError('');setTarget({item,partIndex:index,anchor});
    };
    const canReact=()=>capabilities.allowed('messages.react')&&live.status()==='connected';
    const available=()=>Boolean(current()&&canReact());
    const react=(event:MouseEvent,choice:ReactionChoice)=>{
        if(!event.isTrusted)return;
        const value=current();if(!value||!available()||busy()||unknown())return;
        try{withInput(event,()=>{
            if(!classicReactions.some(reaction=>reaction.kind===choice.kind)&&!(choice.kind==='emoji'&&choice.emoji))return;
            const remove=selected(choice),id=live.command('messages.react',{id:value.item.id,part_index:value.partIndex,reaction:choice.kind,...(choice.kind==='emoji'?{emoji:choice.emoji}:{}),remove});
            intents.set(value.item.id+'/'+value.partIndex+'/'+family(choice),{commandID:id,messageID:value.item.id,chatID:String(value.item.data.chat_id??''),partIndex:value.partIndex,choice:{...choice},remove});
            if(!remove)setFlight({symbol:choice.symbol,color:choice.kind==='love'?'var(--red)':'var(--blue)',from:event.currentTarget instanceof HTMLElement?event.currentTarget.getBoundingClientRect():value.anchor.getBoundingClientRect(),to:value.anchor.getBoundingClientRect(),key:++flightKey});
            changed();close();
        });}catch{setError("Can't react right now. Check your connection.");}
    };
    const checkID=(id:string)=>{if(![...intents.values()].some(intent=>intent.commandID===id))return;try{live.checkCommand(id);}catch{setError("Couldn't check this reaction. Try again.");}};
    const check=()=>{for(const intent of relevant())if(uncertain(command(intent)))checkID(intent.commandID);};
    const notice=(chatID:string)=>{
        revision();const values=[...intents.values()].filter(intent=>intent.chatID===chatID),unknown=values.filter(intent=>uncertain(command(intent))),failed=values.filter(intent=>{const state=command(intent);return state&&!uncertain(state)&&['failed','rejected','expired'].includes(state.status);}),checks=values.filter(intent=>{const state=command(intent);return state&&!observed(state)&&state.check_error;});
        const ids=[...new Set([...unknown,...failed,...checks].map(intent=>intent.commandID))];
        const text=[unknown.length===1?'1 reaction may not have changed':unknown.length?unknown.length+' reactions may not have changed':'',failed.length===1?"1 reaction wasn't changed":failed.length?failed.length+" reactions weren't changed":'',checks.length?"Couldn't check this reaction. Try again.":''].filter(Boolean).join(' · ');
        return ids.length?{count:ids.length,command_ids:ids,text}:undefined;
    };
    const problem=()=>{
        if(error())return error();
        if(relevant().some(intent=>{const state=command(intent);return state&&!observed(state)&&state.check_error;}))return "Couldn't check this reaction. Try again.";
        if(unknown())return 'Not sure if the reaction changed.';
        if(relevant().some(intent=>{const state=command(intent);return state&&['failed','rejected','expired'].includes(state.status);}))return "Reaction wasn't changed.";
        return '';
    };
    const text=()=>{const value=current();if(!value)return '';const part=nativeParts(value.item).find(part=>part.index===value.partIndex);return part?.kind==='text'?part.text??(value.partIndex===0?String(value.item.data.text??''):''):'';};
    const copy=(event:MouseEvent)=>{if(!event.isTrusted)return;try{withInput(event,()=>{void navigator.clipboard.writeText(text()).then(close,()=>setError("Couldn't copy this message."));});}catch{setError("Couldn't copy this message.");}};
    const observe=(items:readonly ThreadItem[],cursor:Cursor|undefined,chatID:string)=>{
        if(!cursor)return;
        const ids=new Set(items.map(item=>item.id));for(const item of items)captured.set(item.id,item);
        for(const [key,intent] of intents){
            if(intent.chatID!==chatID)continue;
            const state=command(intent),same=state?.cursor&&state.cursor.epoch===cursor.epoch&&state.cursor.subscription_revision===cursor.subscription_revision&&state.commit_seq&&cursor.sequence>=state.commit_seq;
            if(state&&final.has(state.status)&&!['indeterminate','delivery_failed'].includes(state.disposition??'')&&!ids.has(intent.messageID)&&target()?.item.id!==intent.messageID){intents.delete(key);continue;}
            if(state&&observed(state)&&state.observed_record_ids?.length&&same){intents.delete(key);if(!ids.has(intent.messageID))captured.delete(intent.messageID);}
        }
        for(const [id] of captured)if(!ids.has(id)&&![...intents.values()].some(intent=>intent.messageID===id)&&target()?.item.id!==id)captured.delete(id);
        changed();
    };
    const dispose=()=>{setTarget();captured.clear();intents.clear();setFlight();};
    onCleanup(dispose);
    return {target:current,open,close,canReact,available,react,selected,unknown,busy,problem,check,checkID,notice,connected:()=>live.status()==='connected',copy,text,observe,flight,clearFlight:()=>setFlight(),dispose};
}
export type MessageActionsController=ReturnType<typeof createMessageActions>;
