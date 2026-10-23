import { createSignal, For, onCleanup, Show, type ComponentProps, type JSX } from 'solid-js';
import type { BridgeRecord, Cursor, ThreadItem } from '../../data/types';
import { auth } from '../../auth/session';
import { createMessageCommands } from '../actions/commands';
import Transcript from './Transcript';
import { createTextMeasurer } from './measure';
import { MessageRow } from './Bubble';
import { layout } from './layout';
export function HooksFixture(){
    const params=new URLSearchParams(location.search),chatID=params.get('chat')??'',commands=createMessageCommands(auth.live()!);
    onCleanup(commands.dispose);
    const [text,setText]=createSignal(''),[baseline,setBaseline]=createSignal(false),[commandID,setCommandID]=createSignal(''),[actionID,setActionID]=createSignal(''),[rawUntouched,setRawUntouched]=createSignal(true),[actionUntouched,setActionUntouched]=createSignal(false),[error,setError]=createSignal('');
    const [media,setMedia]=createSignal<{id:string;messageID:string;rect:{x:number;y:number;width:number;height:number}}>();
    const [typography,setTypography]=createSignal<ReturnType<typeof detachedTypography>>([]);
    const observed=new Map<string,{item:ThreadItem;json:string}>();let priorIDs=new Set<string>(),observedBaseline=false;
    const composer=<section aria-label="Fixture composer"><label>Transcript message<input aria-label="Transcript message" value={text()} onInput={event=>setText(event.currentTarget.value)}/></label><button data-compose-send disabled={!baseline()||!commands.capabilities.allowed('messages.send')} onClick={event=>{try{setCommandID(commands.send(event,{chat_id:chatID,text:text()}));setText('');}catch(error){setError((error as Error).message);}}}>Send transcript message</button></section>;
    const hooks={onOpen:(attachment:BridgeRecord,rect:DOMRectReadOnly)=>setMedia({id:attachment.id,messageID:String(attachment.data.message_id??''),rect:{x:rect.x,y:rect.y,width:rect.width,height:rect.height}}),pending:()=>commands.pending().filter(item=>item.data.chat_id===chatID),projectCaptured:commands.projectCaptured,onObserved:(items:readonly ThreadItem[],cursor:Cursor|undefined)=>{for(const item of items){const key=item.id+':'+item.version,json=JSON.stringify(item);if(!observed.has(key))observed.set(key,{item,json});}commands.observe(items,cursor);const state=window as any;state.__transcriptObservations??=[];state.__transcriptObservations.push({outgoing:observedBaseline?items.filter(item=>item.data.is_from_me&&!priorIDs.has(item.id)).map(item=>item.id):[],frame:state.__transcriptFrame??0,pending:commands.pending().length});priorIDs=new Set(items.map(item=>item.id));if(cursor){observedBaseline=true;setBaseline(true);}setRawUntouched(items.every(item=>item.data.command_id===undefined));},deliveryNotice:()=>commands.notice(chatID),onCheckCommand:commands.check,onCopyCommand:commands.copy,onMessageAction:(_event:Event,item:ThreadItem)=>{setActionID(item.id);const data={...item.data};delete data.command_id;delete data.command_sent_at;setActionUntouched(observed.get(item.id+':'+item.version)?.json===JSON.stringify({...item,data}));},composer};
    return <><Show when={params.get('copy')==='1'}><CopyContractFixture/></Show><div aria-label="Fixture diagnostics" style={{height:'120px',overflow:'auto'}}><button onClick={()=>setRawUntouched([...observed.values()].every(({item,json})=>JSON.stringify(item)===json))}>Inspect observed DTOs</button><button onClick={()=>setTypography(detachedTypography())}>Detach measurement source</button><output aria-label="Native media callback">{JSON.stringify(media()??null)}</output><output aria-label="Detached typography probe">{JSON.stringify(typography())}</output><output aria-label="Transcript connection">{auth.live()!.status()}</output><output aria-label="Transcript baseline">{String(baseline())}</output><output aria-label="Transcript send capability">{String(commands.capabilities.allowed('messages.send'))}</output><output aria-label="Transcript action untouched">{String(actionUntouched())}</output><output aria-label="Transcript raw untouched">{String(rawUntouched())}</output><output aria-label="Transcript command ID">{commandID()}</output><output aria-label="Transcript action ID">{actionID()}</output><output aria-label="Projection timeout count">{commands.timeoutCount()}</output><output role="alert">{error()}</output></div><div class="transcript-body" style={{height:'70dvh'}}><Transcript chatID={chatID} messageID={params.get('anchor')??'latest'} title={()=>undefined} {...hooks}/></div><Show when={commands.notice(chatID)}><output aria-label="Fixture notice source">{commands.notice(chatID)!.text}</output></Show></>;
}

function detachedTypography(){
    const results:{mode:string;expected:number;actual:number;detachedFontSize:string}[]=[];
    for(const mode of ['canvas','features']){
        const source=document.createElement('span'),container=document.createElement('div'),controlHost=document.createElement('div'),detachedHost=document.createElement('div');
        container.style.cssText='position:fixed;left:-10000px;top:0;font:11px serif;visibility:hidden';source.style.cssText='display:inline-block;white-space:pre;font:italic 650 23px InterVariable,sans-serif;letter-spacing:.75px;word-spacing:0px';
        if(mode==='features'){source.style.fontFeatureSettings='"kern" 0,"liga" 0';source.style.fontVariationSettings='"wght" 650';}
        const value='AVATAR office fi 東京';source.textContent=value;container.append(source,controlHost,detachedHost);document.body.append(container);
        const control=createTextMeasurer(),detached=createTextMeasurer();control.prepare(getComputedStyle(source),controlHost);detached.prepare(getComputedStyle(source),detachedHost);control.measure(value);control.flush();const expected=control.measure(value);
        source.remove();detached.measure(value);detached.flush();const actual=detached.measure(value);results.push({mode,expected,actual,detachedFontSize:getComputedStyle(source).fontSize});container.remove();
    }
    return results;
}

function CopyContractFixture(){
    const CopyRow=MessageRow as (props:ComponentProps<typeof MessageRow>&{onCopyCommand:(event:Event,id:string,index?:number)=>void})=>JSX.Element;
    const [copied,setCopied]=createSignal<{id:string;index:number|undefined;trusted:boolean;key:string}[]>([]);
    const inputs=[{id:'multipart',index:10},{id:'multipart',index:2},{id:'legacy',index:undefined},{id:'invalid-fraction',index:1.5},{id:'invalid-negative',index:-1},{id:'invalid-string',index:'2'},{id:'invalid-unsafe',index:Number.MAX_SAFE_INTEGER+1}];
    const items:ThreadItem[]=inputs.map((value,index)=>({id:'command:copy:'+index,collection:'messages.message',source:'messages',version:1,data:{sent_at:'2026-10-20T16:00:00Z',text:'Copy probe '+index,is_from_me:true,command_id:value.id,command_part_index:value.index,send_state:'indeterminate'},reactions:[],attachments:[],reply_count:0}));
    const rows=layout(items,new Map(),{phone:false,group:false,now:new Date('2026-10-20T16:00:00Z'),scale:1,measure:value=>value.length*8,expanded:new Set()},600);
    return <section aria-label="Copy contract probe"><For each={rows}>{row=><CopyRow row={row} edit={()=>undefined} onCopyCommand={(event,id,index)=>{setCopied(value=>[...value,{id,index,trusted:event.isTrusted,key:(row as typeof row&{key?:string}).key??row.id}]);}}/>}</For><output aria-label="Copy contract calls">{JSON.stringify(copied().map(value=>({...value,index:value.index??null,indexType:typeof value.index})))}</output></section>;
}
