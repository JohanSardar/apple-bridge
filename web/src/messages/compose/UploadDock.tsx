import {lazy,Show,Suspense,type JSX} from 'solid-js';
import type {Uploads} from './uploads';
import {Icon} from '../../design/Icon';
import './uploads.css';
const AttachmentQueue=lazy(()=>import('./AttachmentQueue').then(module=>({default:module.AttachmentQueue})));
export function UploadDock(props:{uploads:Uploads;draftID:string;disabled?:boolean;locked?:boolean;children:JSX.Element}){
    let picker!:HTMLInputElement,camera!:HTMLInputElement;
    const add=(_event:Event,files:readonly File[])=>{if(!props.disabled&&!props.locked)props.uploads.add(props.draftID,files);};
    const drop=(event:DragEvent)=>{if(!event.dataTransfer?.files.length)return;event.preventDefault();add(event,Array.from(event.dataTransfer.files));};
    return <div class="upload-dock" onDragOver={event=>{if(event.dataTransfer?.types.includes('Files'))event.preventDefault();}} onDrop={drop} onPaste={event=>{const files=Array.from(event.clipboardData?.files??[]);if(files.length){event.preventDefault();add(event,files);}}}>
        <Show when={props.uploads.busy(props.draftID)}><p class="composer-status" role="status">Preparing attachment…</p></Show><Show when={props.uploads.files(props.draftID).length}><Suspense><AttachmentQueue uploads={props.uploads} draftID={props.draftID}/></Suspense></Show>
        <div class="upload-entry"><button class="icon-button" aria-label="Add attachments" disabled={props.disabled||props.locked} onClick={()=>picker.click()}><Icon name="plus"/></button><button class="icon-button upload-camera" aria-label="Take photo" disabled={props.disabled||props.locked} onClick={()=>camera.click()}><Icon name="camera"/></button><input ref={picker} type="file" multiple hidden aria-label="Choose attachments" onChange={event=>{add(event,Array.from(event.currentTarget.files??[]));event.currentTarget.value='';}}/><input ref={camera} type="file" accept="image/*" capture="environment" hidden aria-label="Camera capture" onChange={event=>{add(event,Array.from(event.currentTarget.files??[]));event.currentTarget.value='';}}/><div class="upload-composer">{props.children}</div></div>
        <Show when={props.uploads.problem()}><p class="composer-error" role="alert">{props.uploads.problem()}</p></Show>
        <Show when={props.disabled&&props.uploads.files(props.draftID).length}><p class="composer-status">Remove attachments to reply to this message</p></Show>
    </div>;
}
