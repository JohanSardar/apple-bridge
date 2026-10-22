import {createSignal} from 'solid-js';
import Transcript from './Transcript';
export function ReplyHandoffFixture(){
    const params=new URLSearchParams(location.search),[replyingTo,setReplyingTo]=createSignal<string>();
    return <><button onClick={()=>setReplyingTo(params.get('anchor')??undefined)}>Set parent reply target</button><div class="transcript-body" style={{height:'75dvh'}}><Transcript chatID={params.get('chat')??''} messageID={params.get('anchor')??'latest'} title={()=>{}} replyingTo={replyingTo} composer={<textarea aria-label="Parent composer"/>}/></div></>;
}
