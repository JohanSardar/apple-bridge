import type {BridgeRecord,ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
export type PlaceholderData={kind:string;title:string;caption:string;hint:string;mapURL?:string;art?:BridgeRecord;variant:'thumb'|'poster'};
const names:Record<string,string>={digital_touch:'Digital Touch',handwriting:'Handwriting',poll:'Poll',check_in:'Check In',apple_cash:'Apple Cash',apple_pay:'Apple Pay',find_my:'Find My',location:'Location',chatbot:'Business message',business_chat:'Business message',rcs:'RCS message',satellite:'via satellite',sos:'SOS'};
function mapURL(raw:unknown):string|undefined {const value=text(raw);if(value.length>4096)return;try{const url=new URL(value),host=url.hostname.replace(/^www\./u,'');if(url.protocol==='https:'&&!url.username&&!url.password&&(host==='maps.apple.com'||host==='maps.google.com'||host==='google.com'&&url.pathname.startsWith('/maps')))return url.href;}catch{}return;}
export function messagePlaceholderData(message:ThreadItem):PlaceholderData {
 const balloon=fields(message.data.balloon),kind=text(balloon.kind).slice(0,128)||'unknown',payload=fields(balloon.app),title=(Object.hasOwn(names,kind)?names[kind]:text(balloon.app_name).trim()||'Unsupported message').slice(0,128),rawCaption=(text(payload.caption)||text(payload.subcaption)||text(message.data.text)).trim().slice(0,512),caption=rawCaption.toLowerCase()===title.trim().toLowerCase()?'':rawCaption,url=['find_my','location'].includes(kind)?mapURL(payload.url):undefined;
 const referenced=message.attachments.find(attachment=>attachment.id===text(payload.image_attachment_id)),mime=text(referenced?.data.mime_type),variant=mime.startsWith('video/')?'poster':'thumb',derivative=fields(fields(referenced?.data.derivatives)[variant]);
 const art=referenced&&/^(image|video)\//u.test(mime)&&/^[a-f0-9]{64}$/u.test(text(derivative.digest))?referenced:undefined;
 return {kind,title,caption,hint:url?'Open in Maps':'Open on an Apple device',mapURL:url,art,variant};
}
