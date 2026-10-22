import {For,Show} from 'solid-js';
import {PendingMedia} from '../media/PendingMedia';
import type {Uploads} from './uploads';
export function AttachmentQueue(props:{uploads:Uploads;draftID:string}){
    return <><Show when={props.uploads.files(props.draftID).length}><ul class="upload-files" aria-label="Attachments"><For each={props.uploads.files(props.draftID)}>{entry=><li data-upload-key={entry.key} data-upload-id={entry.remote?.upload_id} data-upload-state={entry.phase}>
            <div class="upload-preview" aria-hidden="true"><PendingMedia attachment={entry.attachment} width={100} height={76}/></div><div class="upload-description"><strong>{entry.file.name}</strong><progress max={entry.file.size||1} value={entry.bytes} aria-label={'Upload '+entry.file.name}/><span role="status">{props.uploads.expired(entry)?'Upload expired. Retry to continue':entry.phase==='ready'?'Ready to send':entry.phase==='paused'?entry.problem:'Uploading…'}</span></div>
            <Show when={entry.phase==='uploading'||entry.phase==='finishing'} fallback={<Show when={entry.phase==='paused'||props.uploads.expired(entry)}><button class="button subtle" onClick={()=>props.uploads.retry(entry)}>Retry upload</button></Show>}><button class="button subtle" onClick={()=>props.uploads.pause(entry)}>Pause upload</button></Show><button class="icon-button" aria-label={'Remove '+entry.file.name} onClick={()=>props.uploads.remove(props.draftID,entry.key)}>×</button>
        </li>}</For></ul></Show></>;
}
