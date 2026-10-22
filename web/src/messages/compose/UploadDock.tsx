import {For,Show,type JSX} from 'solid-js';
import {PendingMedia} from '../media/PendingMedia';
import type {Uploads} from './uploads';
import './uploads.css';
export function UploadDock(props:{uploads:Uploads;draftID:string;disabled?:boolean;locked?:boolean;children:JSX.Element}){
    let picker!:HTMLInputElement,camera!:HTMLInputElement;
    const add=(_event:Event,files:readonly File[])=>{if(!props.disabled&&!props.locked)props.uploads.add(props.draftID,files);};
    const drop=(event:DragEvent)=>{if(!event.dataTransfer?.files.length)return;event.preventDefault();add(event,Array.from(event.dataTransfer.files));};
    return <div class="upload-dock" onDragOver={event=>{if(event.dataTransfer?.types.includes('Files'))event.preventDefault();}} onDrop={drop} onPaste={event=>{const files=Array.from(event.clipboardData?.files??[]);if(files.length){event.preventDefault();add(event,files);}}}>
        <Show when={props.uploads.busy(props.draftID)}><p class="composer-status" role="status">Preparing attachment…</p></Show><Show when={props.uploads.files(props.draftID).length}><ul class="upload-files" aria-label="Attachments"><For each={props.uploads.files(props.draftID)}>{entry=><li data-upload-key={entry.key} data-upload-id={entry.remote?.upload_id} data-upload-state={entry.phase}>
            <div class="upload-preview" aria-hidden="true"><PendingMedia attachment={entry.attachment} width={100} height={76}/></div><div class="upload-description"><strong>{entry.file.name}</strong><progress max={entry.file.size||1} value={entry.bytes} aria-label={'Upload '+entry.file.name}/><span role="status">{props.uploads.expired(entry)?'Upload expired. Retry to continue':entry.phase==='ready'?'Ready to send':entry.phase==='paused'?entry.problem:'Uploading…'}</span></div>
            <Show when={entry.phase==='uploading'||entry.phase==='finishing'} fallback={<Show when={entry.phase==='paused'||props.uploads.expired(entry)}><button class="button subtle" onClick={()=>props.uploads.retry(entry)}>Retry upload</button></Show>}><button class="button subtle" onClick={()=>props.uploads.pause(entry)}>Pause upload</button></Show><button class="icon-button" aria-label={'Remove '+entry.file.name} onClick={()=>props.uploads.remove(props.draftID,entry.key)}>×</button>
        </li>}</For></ul></Show>
        <div class="upload-entry"><button class="icon-button" aria-label="Add attachments" disabled={props.disabled||props.locked} onClick={()=>picker.click()}>＋</button><button class="icon-button upload-camera" aria-label="Take photo" disabled={props.disabled||props.locked} onClick={()=>camera.click()}>◉</button><input ref={picker} type="file" multiple hidden aria-label="Choose attachments" onChange={event=>{add(event,Array.from(event.currentTarget.files??[]));event.currentTarget.value='';}}/><input ref={camera} type="file" accept="image/*" capture="environment" hidden aria-label="Camera capture" onChange={event=>{add(event,Array.from(event.currentTarget.files??[]));event.currentTarget.value='';}}/><div class="upload-composer">{props.children}</div></div>
        <Show when={props.uploads.problem()}><p class="composer-error" role="alert">{props.uploads.problem()}</p></Show>
        <Show when={props.disabled&&props.uploads.files(props.draftID).length}><p class="composer-status">Remove attachments to reply to this message</p></Show>
    </div>;
}
