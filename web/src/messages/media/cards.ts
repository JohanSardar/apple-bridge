import type { ThreadItem } from '../../data/types';
import { fields, text } from '../store/types';
export type MessageCardData = {kind:'link'|'app';title:string;domain:string;appName:string;caption:string;subcaption:string;url?:string;imageAttachmentId:string;hasImage:boolean;video:boolean};
function webURL(value:unknown):URL|undefined {
    try {const url=new URL(text(value));return /^https?:$/u.test(url.protocol)&&Boolean(url.hostname)?url:undefined;} catch {return;}
}
export function messageCardData(message:ThreadItem):MessageCardData|undefined {
    const balloon=fields(message.data.balloon),kind=text(balloon.kind);
    if(kind!=='link'&&kind!=='app')return;
    const payload=fields(balloon[kind]),url=webURL(payload.url),domain=url?.hostname.replace(/^www\./u,'')??'',appName=text(balloon.app_name)||'iMessage app',caption=text(payload.caption),imageAttachmentId=text(payload.image_attachment_id);
    const attachment=message.attachments.find(value=>value.id===imageAttachmentId),localVideo=text(attachment?.data.mime_type).startsWith('video/');
    return {kind,title:kind==='link'?text(payload.title)||domain||'Link':caption||appName,domain,appName,caption,subcaption:text(payload.subcaption)||text(payload.trailing_caption),url:url?.href,imageAttachmentId,hasImage:Boolean(imageAttachmentId),video:localVideo||Boolean(webURL(payload.video_url))};
}
