import {createEffect,createMemo,createSignal,For,onCleanup,onMount,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {Icon} from '../../design/Icon';
import {attachmentFilename,attachmentOriginal,saveAttachment} from './files';
import {readVCard,type VCard} from './vcard';
import './files.css';
function Photo(props:{source?:string}){return <span class="media-contact-photo"><Icon name="contacts"/><Show when={props.source}>{source=><img src={source()} alt="Contact photo" decoding="async" draggable={false} onLoad={event=>event.currentTarget.hidden=false} onError={event=>event.currentTarget.hidden=true}/>}</Show></span>;}
function ContactDetails(props:{contact:VCard;photo?:string;onClose:()=>void}){
 let dialog!:HTMLDialogElement;onMount(()=>dialog.showModal());onCleanup(()=>dialog.close());
 return <dialog ref={dialog} class="media-contact-details" aria-label={props.contact.name} onClose={props.onClose}><button type="button" class="media-contact-close" aria-label="Close contact" title="Close contact" onClick={()=>dialog.close()}><Icon name="close"/></button><Photo source={props.photo}/><h2>{props.contact.name}</h2><dl><For each={props.contact.fields}>{field=><div><dt>{field.label}</dt><dd>{field.value}</dd></div>}</For></dl></dialog>;
}
export function ContactCard(props:{attachment:BridgeRecord}){
 const original=createMemo(()=>attachmentOriginal(props.attachment));
 const [contact,setContact]=createSignal<VCard>(),[photo,setPhoto]=createSignal<string>(),[failed,setFailed]=createSignal(false),[attempt,setAttempt]=createSignal(0),[view,setView]=createSignal(false);let trigger!:HTMLButtonElement,generation=0,disposed=false,photoURL:string|undefined;
 createEffect(()=>{const source=original();attempt();const token=++generation,controller=new AbortController();setContact(undefined);setFailed(false);const previous=photoURL;if(previous)URL.revokeObjectURL(previous);photoURL=undefined;setPhoto(undefined);if(!source){setFailed(true);return;}void readVCard(source,controller.signal).then(value=>{if(disposed||token!==generation)return;setContact(value);if(value.photo){photoURL=URL.createObjectURL(new Blob([new Uint8Array(value.photo.bytes).buffer],{type:value.photo.type}));setPhoto(photoURL);}}).catch(()=>{if(!disposed&&token===generation&&!controller.signal.aborted)setFailed(true);});onCleanup(()=>controller.abort());});
 onCleanup(()=>{disposed=true;generation++;if(photoURL)URL.revokeObjectURL(photoURL);});
 const closed=()=>{setView(false);queueMicrotask(()=>{if(trigger?.isConnected)trigger.focus({preventScroll:true});});};
 return <div class="media-file-chip media-contact-card" role="group" aria-label={contact()?.name??'Contact Card'} data-contact-card><Photo source={photo()}/><span class="media-file-copy"><strong title={contact()?.name}>{contact()?.name??'Contact Card'}</strong><span title={attachmentFilename(props.attachment)}>{contact()?.name&&contact()?.name!=='Contact Card'?'Contact Card':attachmentFilename(props.attachment)}</span><span role="status">{failed()?'Contact unavailable':contact()?'View contact':'Loading contact'}</span></span><button ref={trigger} type="button" class="media-contact-open" aria-label={failed()&&original()?'Try again':'View contact'} disabled={failed()?!original():!contact()} onClick={()=>{if(failed())setAttempt(value=>value+1);else setView(true);}}/><Show when={original()}>{url=><a class="media-file-download" href={url()} download={attachmentFilename(props.attachment)} aria-label="Download contact" onClick={event=>{event.preventDefault();saveAttachment(props.attachment);}} title="Download original contact"><Icon name="download"/></a>}</Show><Show when={view()&&contact()}>{value=><ContactDetails contact={value()} photo={photo()} onClose={closed}/>}</Show></div>;
}
