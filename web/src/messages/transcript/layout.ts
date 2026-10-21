import type { BridgeRecord, ThreadItem } from '../../data/types';
import { array, fields, messageParts, sender, text, type Handles, type Part } from '../store/types';
import { groupEvent, headerParts, name, time, wording, type HeaderParts } from '../format';
import { attachmentHasAlpha } from '../media/source';
export const transcriptSpacing={withinRun:3,betweenRuns:9,partGap:3,dateLine:16,afterHeader:9,senderLine:16,senderNameGap:2};
export type LayoutPrefs = { phone: boolean; viewportHeight?:number; group: boolean; service?: string; atLatest?: boolean; now: Date; locale?: string; scale: number; measure: (value:string,font?:{weight?:number;italic?:boolean})=>number; expanded: ReadonlySet<string> };
export type BubbleLayout = { part: Part; subject?:string; width: number; height: number; emoji: boolean; media: boolean; floating:boolean; reactions: BridgeRecord[]; tail: boolean };
export type TranscriptRow = { id: string; item: ThreadItem; height: number; bubbles: BubbleLayout[]; sent: boolean; sender: string; senderName?: string; avatar?: string; header?: string; headerParts?:HeaderParts; system?: string; receipt?: string; edited: boolean; history: {text:string;height:number;width:number}[]; unsent?:string; reply?: string; failed: boolean; gap: number };
const emojiPattern = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|\p{Emoji_Modifier}|\uFE0F|\u200D|\u20E3|[0-9#*]\uFE0F?\u20E3|\s)+$/u;
export function emojiOnly(value: string) {
    if(!emojiPattern.test(value)||!/(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[0-9#*]\uFE0F?\u20E3)/u.test(value))return false;
    const count=Array.from(new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(value.replace(/\s/gu,''))).length;
    return count>=1&&count<=3;
}
export function wrapText(value: string, maxWidth: number, measure: LayoutPrefs['measure']) {
    let line=0,lines=1,widest=0;
    const tokens=value.split(/(\s+)/u);
    for(const token of tokens){
        if(token.includes('\n')){const chunks=token.split('\n');for(let i=0;i<chunks.length;i++){line+=measure(chunks[i]);if(i<chunks.length-1){widest=Math.max(widest,line);line=0;lines++;}}continue;}
        const width=measure(token);
        if(width>maxWidth){for(const character of Array.from(token)){const advance=measure(character);if(line+advance>maxWidth&&line>0){widest=Math.max(widest,line);line=0;lines++;}line+=advance;}continue;}
        if(line+width>maxWidth&&line>0){widest=Math.max(widest,line);line=/^\s+$/u.test(token)?0:width;lines++;}else line+=width;
    }
    return {width:Math.min(maxWidth,Math.max(widest,line)),lines};
}
function mediaSize(dimensions:Record<string,unknown>,maxWidth:number,prefs:LayoutPrefs,alpha:boolean) {
    const width=Number(dimensions.width),height=Number(dimensions.height);
    if(!Number.isFinite(width)||!Number.isFinite(height)||width<=0||height<=0)return {width:Math.min(maxWidth,260),height:88};
    const maxHeight=prefs.phone?(prefs.viewportHeight??844)*.6:360,scale=Math.min(1,maxWidth/width,prefs.phone&&!alpha?Infinity:maxHeight/height);
    return {width:width*scale,height:Math.min(height*scale,maxHeight)};
}
function partSize(item:ThreadItem,part:Part,maxWidth:number,lineHeight:number,prefs:LayoutPrefs){
    if(part.kind==='genmoji')return {width:lineHeight,height:lineHeight};
    const attachment=item.attachments?.find(value=>value.id===part.attachment_id),mime=text(attachment?.data.mime_type);
    if(mime.startsWith('audio/'))return {width:Math.min(maxWidth,280),height:76+(text(fields(item.data.audio).transcription).trim()?44:0)};
    if(attachment&&!/^(image|video)\//u.test(mime))return {width:Math.min(maxWidth,260),height:88};
    if(attachment)return mediaSize(fields(attachment.data.media),maxWidth,prefs,attachmentHasAlpha(attachment));
    const balloon=fields(item.data.balloon),kind=text(balloon.kind);
    if(kind==='link'||kind==='app')return {width:Math.min(maxWidth,280),height:fields(balloon[kind]).image_attachment_id?180:88};
    return {width:Math.min(maxWidth,260),height:88};
}
function sameRun(a: ThreadItem|undefined,b: ThreadItem|undefined) { return !!a&&!!b&&a.data.kind!=='group_event'&&b.data.kind!=='group_event'&&!array(a.data.retracted_parts).length&&!array(b.data.retracted_parts).length&&Boolean(a.data.is_from_me)===Boolean(b.data.is_from_me)&&sender(a)===sender(b)&&Math.abs(Date.parse(b.data.sent_at)-Date.parse(a.data.sent_at))<=60000; }
export function layout(items: readonly ThreadItem[], handles: Handles, prefs: LayoutPrefs, width: number): TranscriptRow[] {
    const fontSize=(prefs.phone?17:13)*prefs.scale,lineHeight=(prefs.phone?22:16)*prefs.scale;
    const maxWidth=Math.max(60,Math.min(prefs.phone?width*.7:480,width-(prefs.group?80:40))), padding=24;
    const lastSent=items.findLastIndex(item=>Boolean(item.data.is_from_me)&&item.data.kind!=='group_event'&&!item.data.retracted_at&&messageParts(item).some(part=>!array(item.data.retracted_parts).includes(part.index)));
    return items.map((item,index)=>{
        const data=item.data,sent=Boolean(data.is_from_me),actor=sent?wording.you:name(text(fields(data.group_event).actor_handle_id)||sender(item),handles),previous=items[index-1],next=items[index+1],first=!sameRun(previous,item),last=!sameRun(item,next);
        const dateParts=!previous||Date.parse(data.sent_at)-Date.parse(previous.data.sent_at)>3600000?headerParts(data.sent_at,prefs.now,prefs.locale):undefined,dateHeader=dateParts?`${dateParts.day}${dateParts.separator}${dateParts.time}`:undefined;
        const unsent=array(data.retracted_parts),parts=messageParts(item),visible=parts.filter(part=>!unsent.includes(part.index));
        const system=data.kind==='group_event'?groupEvent(data.group_event,actor,handles):data.retracted_at||visible.length===0?wording.unsent(actor):undefined;
        const bubbles: BubbleLayout[]=system?[]:visible.map((part,partIndex)=>{
            const emoji=part.kind==='text'&&emojiOnly(part.text??''),media=!['text','mention'].includes(part.kind);
            const content=text(part.text)||text(data.text),wrapped=wrapText(content,maxWidth-padding,prefs.measure);
            const size=partSize(item,part,maxWidth,lineHeight,prefs),subject=partIndex===0&&!media?text(data.subject):'',subjectSize=wrapText(subject,maxWidth-padding,value=>prefs.measure(value,{weight:700}));
            const bubbleWidth=media?size.width:emoji?Math.min(maxWidth,Math.ceil(prefs.measure(content)*2.7)+4):Math.max(36,Math.ceil(Math.max(wrapped.width,subject?subjectSize.width:0))+padding);
            const bubbleHeight=media?size.height:emoji?Math.ceil(fontSize*3):wrapped.lines*lineHeight+14+(subject?subjectSize.lines*lineHeight:0);
            const attachment=item.attachments?.find(value=>value.id===part.attachment_id),floating=part.kind==='genmoji'||Boolean(attachment&&attachmentHasAlpha(attachment));
            return {part,floating,subject:subject||undefined,width:bubbleWidth,height:bubbleHeight,emoji,media,reactions:(item.reactions??[]).filter(reaction=>Number(reaction.data.part_index??0)===part.index),tail:last&&partIndex===visible.length-1&&!emoji};
        });
        const edited=Boolean(data.edited_at)||array(data.edit_history).length>0,history=prefs.expanded.has(item.id)?array(data.edit_history).map(value=>{const content=text(fields(value).text);const wrapped=wrapText(content,maxWidth-padding,prefs.measure);return {text:content,width:Math.max(36,Math.ceil(wrapped.width)+padding),height:wrapped.lines*lineHeight+14};}):[];
        const receipt=index===lastSent&&prefs.atLatest!==false&&Number(data.error_code??0)===0?(data.read_at?`Read ${time(text(data.read_at),prefs.locale)}`:data.delivered_at?fields(data.delivery).quietly?wording.deliveredQuietly:wording.delivered:undefined):undefined;
        const reply=text(data.thread_originator_id),failed=Number(data.error_code??0)!==0;
        const gap=first?transcriptSpacing.betweenRuns:transcriptSpacing.withinRun,senderName=prefs.group&&!sent&&first&&!system?actor:undefined;
        let height=gap+(dateHeader?transcriptSpacing.dateLine+transcriptSpacing.afterHeader:0)+(senderName?transcriptSpacing.senderLine+transcriptSpacing.senderNameGap:0)+(reply&&reply!==item.id?26:0);
        height+=system?30:bubbles.reduce((sum,bubble,index)=>sum+bubble.height+(bubble.reactions.length?16:0)+(index?transcriptSpacing.partGap:0),0);
        height+=(edited?22:0)+(receipt?20:0)+(failed?20:0)+(item.reply_count?24:0)+history.reduce((sum,version)=>sum+version.height+4,0)+(unsent.length&&visible.length?24:0);
        return {id:item.id,item:data.service?item:{...item,data:{...data,service:prefs.service??'iMessage'}},height:Math.ceil(height),bubbles,sent,sender:actor,senderName,avatar:prefs.group&&!sent&&last&&!system?handles.get(sender(item))?.initials??'?':undefined,header:dateHeader,headerParts:dateParts,system,receipt,edited,history,reply:reply&&reply!==item.id?reply:undefined,failed,gap,unsent:unsent.length&&visible.length?wording.unsent(actor):undefined};
    });
}
