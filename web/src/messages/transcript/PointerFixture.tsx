import {createSignal} from 'solid-js';
import Transcript from './Transcript';
export function PointerFixture(){
    const params=new URLSearchParams(location.search),[enabled,setEnabled]=createSignal(false),[events,setEvents]=createSignal<object[]>([]),[actions,setActions]=createSignal(0);
    return <><button onClick={()=>setEnabled(value=>!value)}>Toggle pointer hook</button><output aria-label="Pointer events">{JSON.stringify(events())}</output><output aria-label="Message actions">{actions()}</output><div class="transcript-body" style={{height:'75dvh'}}><Transcript chatID={params.get('chat')??''} messageID={params.get('anchor')??'latest'} title={()=>{}} onMessageAction={()=>setActions(value=>value+1)} onMessagePointer={enabled()?(event,item)=>setEvents(values=>[...values,{type:event.type,pointerType:event.pointerType,id:item.id,version:item.version,prevented:event.defaultPrevented}]):undefined}/></div></>;
}
