import { dateLabel, wording, name, groupEvent } from './index';
import { text } from '../store/types';
import type { ChatSummary } from '../../data/types';
import { strings as s } from '../../app/strings';
import type { Handles } from '../store/types';
const emptyHandles:Handles=new Map();
const dateFormats=new Map<string,Intl.DateTimeFormat>();
const dateOptions:Record<'time'|'weekday'|'date',Intl.DateTimeFormatOptions>={time:{hour:'numeric',minute:'2-digit'},weekday:{weekday:'long'},date:{year:'2-digit',month:'numeric',day:'numeric'}};
function dateFormat(kind:keyof typeof dateOptions,locale?:string){
    const key=`${locale??''}:${kind}`;
    let format=dateFormats.get(key);
    if(!format){format=new Intl.DateTimeFormat(locale,dateOptions[kind]);if(dateFormats.size>=24)dateFormats.delete(dateFormats.keys().next().value!);dateFormats.set(key,format);}
    return format;
}

export function conversationDate(value: string, now=new Date(), locale?:string) {
    if(!value||!Number.isFinite(Date.parse(value)))return '';
    const date=new Date(value), today=new Date(now.getFullYear(),now.getMonth(),now.getDate()), day=new Date(date.getFullYear(),date.getMonth(),date.getDate());
    const days=Math.round((today.getTime()-day.getTime())/86400000);
    if(days===0)return dateFormat('time',locale).format(date);
    if(days===1)return dateLabel(value,now,locale);
    if(days>1&&days<7)return dateFormat('weekday',locale).format(date);
    return dateFormat('date',locale).format(date);
}
export function preview(last: import('../../data/types').ChatSummary['last_message'], handles:Handles=emptyHandles) {
    if(!last)return '';
    const value=last.preview;
    const actor=last.is_from_me?wording.you:last.sender_handle_id&&handles.has(last.sender_handle_id)?name(last.sender_handle_id,handles):s.someone;
    if(value.kind==='unsent')return wording.unsent(actor);
    if(value.kind==='group_event')return groupEvent(value.group_event,actor,handles);
    if(value.kind==='reaction')return value.reaction?(value.reaction.removed||value.reaction.verb==='Removed reaction from'?reactionRemoval(value.reaction.kind??'',value.reaction.target_excerpt):`${value.reaction.verb} “${value.reaction.target_excerpt}”`):'';
    if(value.kind==='attachments'){
        const count=value.count??1, kinds=value.attachment_kinds??[];
        if(kinds.length===1&&/image|photo/iu.test(kinds[0]))return s.attachmentImages(count);
        if(kinds.length===1)return ({video:s.video,videos:s.video,audio:s.audioMessage,location:s.location,contact:s.contactCard,vcard:s.contactCard,sticker:s.sticker,stickers:s.sticker,link:s.link} as Record<string,string>)[kinds[0]]??s.attachmentFiles(count);
        return s.attachmentFiles(count);
    }
    return value.text??'';
}

export function avatarInitials(title:string,given?:string,family?:string){
    const structured=Boolean(given||family);
    if(!structured&&/@/u.test(title))return '';
    const words=title.trim().split(/\s+/u),first=structured?given??'':words[0]??'',last=structured?family??'':words.length>1?words.at(-1)!:'';
    return [first,last].map(word=>/^\p{L}/u.test(word.trim())?Array.from(word.trim())[0]:'').join('').toLocaleUpperCase();
}
export function explicitlyMuted(value:unknown){return value===true;}
export function unnamedGroupTitle(titles:string[],structured?:readonly(string|undefined)[]){
    const given=titles.map((title,index)=>structured?.[index]||title.trim().split(/\s+/u)[0]).filter(Boolean);
    return given.length<3?given.join(' & '):`${given[0]}, ${given[1]} & ${given.length-2} more`;
}
export function reactionRemoval(kind:string,excerpt:string){
    const noun=({love:'a heart',like:'a like',dislike:'a thumbs down',laugh:'a laugh',emphasize:'an emphasis',question:'a question mark'} as Record<string,string>)[kind]??'a reaction';
    return `Removed ${noun} from “${excerpt}”`;
}
export function conversationTitle(chat:ChatSummary,handles:Handles){
    if(text(chat.chat.data.display_name))return text(chat.chat.data.display_name);
    const ids=Array.isArray(chat.chat.data.participants)?chat.chat.data.participants.filter((id):id is string=>typeof id==='string'):[];
    const people=ids.map(id=>handles.get(id));
    if(ids.length<2||people.some(person=>!person))return chat.title;
    return unnamedGroupTitle(people.map(person=>person!.display_name),people.map(person=>person!.given_name));
}
