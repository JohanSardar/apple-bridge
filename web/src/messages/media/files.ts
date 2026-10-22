import type {BridgeRecord} from '../../data/types';
import {fields,text} from '../store/types';
import {mediaSource} from './source';
export function attachmentIsContact(attachment:BridgeRecord):boolean {return /^(text\/(x-)?vcard)$/iu.test(text(attachment.data.mime_type))||text(attachment.data.uti)==='public.vcard'||/\.vcf$/iu.test(attachmentFilename(attachment));}
export function attachmentFilename(attachment:BridgeRecord){return text(attachment.data.filename).split(/[\\/]/u).at(-1)?.replace(/[\u0000-\u001f\u007f]/gu,'').trim().slice(0,255)||'Attachment';}
export function attachmentOriginal(attachment:BridgeRecord){return fields(attachment.data.blob).state==='present'?mediaSource(attachment,'original'):undefined;}
export function attachmentSize(attachment:BridgeRecord){const value=Number(attachment.data.size);if(!Number.isFinite(value)||value<0)return '';const unit=value>=1e9?['GB',1e9] as const:value>=1e6?['MB',1e6] as const:value>=1e3?['KB',1e3] as const:['bytes',1] as const;return new Intl.NumberFormat(undefined,{maximumFractionDigits:unit[1]===1?0:1}).format(value/unit[1])+' '+unit[0];}
export function saveAttachment(attachment:BridgeRecord):boolean {const source=attachmentOriginal(attachment);if(!source)return false;const link=document.createElement('a');link.href=source;link.download=attachmentFilename(attachment);link.rel='noopener noreferrer';link.hidden=true;document.body.appendChild(link);link.click();link.remove();return true;}
