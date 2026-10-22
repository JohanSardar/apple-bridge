import {Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {fields,text} from '../store/types';
import {Icon} from '../../design/Icon';
import {attachmentFilename,attachmentOriginal,attachmentSize,saveAttachment} from './files';
import './files.css';
export function FileChip(props:{attachment:BridgeRecord}){
 const filename=()=>attachmentFilename(props.attachment),source=()=>attachmentOriginal(props.attachment),extension=()=>/\.[^.]+$/u.exec(filename())?.[0].slice(1,5).toUpperCase()||'FILE';
 return <div class="media-file-chip" role="group" aria-label={filename()} data-file-mime={text(props.attachment.data.mime_type)} data-file-digest={text(fields(props.attachment.data.blob).digest)}><span class="media-file-glyph" aria-hidden="true"><svg viewBox="0 0 36 44"><path d="M5 1h17l9 9v32H5ZM22 1v10h9"/><text x="18" y="30" text-anchor="middle">{extension()}</text></svg></span><span class="media-file-copy"><strong title={filename()}>{filename()}</strong><span>{attachmentSize(props.attachment)}</span><Show when={!source()}><span class="media-file-status">File unavailable</span></Show></span><Show when={source()}>{url=><><a class="media-file-open" href={url()} target="_blank" rel="noopener noreferrer" aria-label="Open file" title={'Open '+filename()}/><a class="media-file-download" href={url()} download={filename()} aria-label="Download file" onClick={event=>{event.preventDefault();saveAttachment(props.attachment);}} title={'Download '+filename()}><Icon name="download"/></a></>}</Show></div>;
}
