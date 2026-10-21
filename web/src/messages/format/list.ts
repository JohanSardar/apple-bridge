import { time, dateLabel, wording, name, groupEvent } from './index';
import { strings as s } from '../../app/strings';
import type { Handles } from '../store/types';
const emptyHandles:Handles=new Map();

export function conversationDate(value: string, now=new Date(), locale?:string) {
    if(!value||!Number.isFinite(Date.parse(value)))return '';
    const date=new Date(value), today=new Date(now.getFullYear(),now.getMonth(),now.getDate()), day=new Date(date.getFullYear(),date.getMonth(),date.getDate());
    const days=Math.round((today.getTime()-day.getTime())/86400000);
    if(days===0)return time(value,locale);
    if(days>0&&days<7)return dateLabel(value,now,locale);
    return new Intl.DateTimeFormat(locale,{year:'2-digit',month:'numeric',day:'numeric'}).format(date);
}
export function preview(last: import('../../data/types').ChatSummary['last_message'], handles:Handles=emptyHandles) {
    if(!last)return '';
    const value=last.preview;
    const actor=last.is_from_me?wording.you:last.sender_handle_id&&handles.has(last.sender_handle_id)?name(last.sender_handle_id,handles):s.someone;
    if(value.kind==='unsent')return wording.unsent(actor);
    if(value.kind==='group_event')return groupEvent(value.group_event,actor,handles);
    if(value.kind==='reaction')return value.reaction?`${value.reaction.verb} “${value.reaction.target_excerpt}”`:'';
    if(value.kind==='attachments'){
        const count=value.count??1, kinds=value.attachment_kinds??[];
        if(kinds.length===1&&/image|photo/iu.test(kinds[0]))return s.attachmentImages(count);
        if(kinds.length===1)return ({video:s.video,videos:s.video,audio:s.audioMessage,location:s.location,contact:s.contactCard,vcard:s.contactCard,sticker:s.sticker,stickers:s.sticker,link:s.link} as Record<string,string>)[kinds[0]]??s.attachmentFiles(count);
        return s.attachmentFiles(count);
    }
    return value.text??'';
}
