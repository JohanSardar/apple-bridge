import type { BridgeRecord, ThreadItem } from '../../data/types';
import { array, fields, messageParts, sender, text, type Handles, type Part } from '../store/types';
import { groupEvent, headerParts, name, time, wording, type HeaderParts } from '../format';
import { attachmentHasAlpha } from '../media/source';
import {pendingGeometryKnown,type PendingAttachment} from '../media/pending';
import {effectLabel} from '../effects/data';
import {messageCardData} from '../media/cards';
export const effectLineHeight=22;
export const transcriptSpacing={withinRun:3,betweenRuns:9,partGap:3,dateLine:16,afterHeader:9,senderLine:16,senderNameGap:2,reactionTop:16};
export type LayoutPrefs = { phone: boolean; reduceMotion?:boolean; selectedMessageID?:string; viewportHeight?:number; group: boolean; service?: string; atLatest?: boolean; now: Date; locale?: string; scale: number; measure: (value:string,font?:{weight?:number;italic?:boolean})=>number; expanded: ReadonlySet<string> };
export type BubbleLayout = { part: Part; pendingAttachment?:PendingAttachment; mediaAttachments?:readonly BridgeRecord[]; subject?:string; width: number; height: number; emoji: boolean; media: boolean; floating:boolean; reactions: BridgeRecord[]; tail: boolean };
export type TranscriptRow = { key:string;service:string;effectLabel?:string;commandStatus?:{id:string;state:string;label:string};id: string; item: ThreadItem; members?:readonly ThreadItem[]; height: number; bubbles: BubbleLayout[]; sent: boolean; sender: string; senderName?: string; avatar?: string; header?: string; headerParts?:HeaderParts; system?: string; receipt?: string; edited: boolean; history: {text:string;height:number;width:number}[]; unsent?:string; reply?: string; failed: boolean; gap: number };
const emojiPattern = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|\p{Emoji_Modifier}|\uFE0F|\u200D|\u20E3|[0-9#*]\uFE0F?\u20E3|\s)+$/u;
let graphemes:Intl.Segmenter|undefined;
export function emojiOnly(value: string) {
    if(!emojiPattern.test(value)||!/(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[0-9#*]\uFE0F?\u20E3)/u.test(value))return false;
    const count=Array.from((graphemes??=new Intl.Segmenter(undefined,{granularity:'grapheme'})).segment(value.replace(/\s/gu,''))).length;
    return count>=1&&count<=3;
}
export function wrapText(value: string, maxWidth: number, measure: LayoutPrefs['measure']) {
    let line=0,lines=1,widest=0;
    const tokens=value.split(/(\s+)/u);
    for(const token of tokens){
        if(token.includes('\n')){const chunks=token.split('\n');for(let i=0;i<chunks.length;i++){line+=measure(chunks[i]);if(i<chunks.length-1){widest=Math.max(widest,line);line=0;lines++;}}continue;}
        const width=measure(token);
        if(textWidthWithSlack(width)>maxWidth){for(const character of Array.from(token)){const advance=measure(character);if(textWidthWithSlack(line+advance)>maxWidth&&line>0){widest=Math.max(widest,line);line=0;lines++;}line+=advance;}continue;}
        if(textWidthWithSlack(line+width)>maxWidth&&line>0){widest=Math.max(widest,line);line=/^\s+$/u.test(token)?0:width;lines++;}else line+=width;
    }
    return {width:Math.min(maxWidth,Math.max(widest,line)),lines};
}
export function textWidthWithSlack(width:number){return Math.ceil(Math.ceil(width)+Math.max(1,width*.005));}
function mediaHeightCap(prefs:LayoutPrefs){return prefs.phone?(prefs.viewportHeight??844)*.6:360;}
function mediaSize(dimensions:Record<string,unknown>,maxWidth:number,prefs:LayoutPrefs,alpha:boolean) {
    const width=Number(dimensions.width),height=Number(dimensions.height);
    if(!Number.isFinite(width)||!Number.isFinite(height)||width<=0||height<=0)return {width:Math.min(maxWidth,260),height:88};
    const maxHeight=mediaHeightCap(prefs),scale=Math.min(1,maxWidth/width,prefs.phone&&!alpha?Infinity:maxHeight/height);
    return {width:width*scale,height:Math.min(height*scale,maxHeight)};
}
function pendingAttachment(item:ThreadItem):PendingAttachment|undefined{return item.id.startsWith('command:')&&typeof item.data.command_id==='string'?array(item.data.local_attachments)[0] as PendingAttachment|undefined:undefined;}
function partSize(item:ThreadItem,part:Part,maxWidth:number,lineHeight:number,prefs:LayoutPrefs){
    const local=pendingAttachment(item);
    if(local){if(!/^(image|video)\//u.test(local.type)||!pendingGeometryKnown(local))return {width:Math.min(maxWidth,260),height:local.type.startsWith('video/')?120:88};const size=mediaSize({width:local.width,height:local.height},maxWidth,prefs,local.alpha===true);return local.type.startsWith('video/')?{width:Math.min(maxWidth,Math.max(120,size.width)),height:Math.min(mediaHeightCap(prefs),Math.max(120,size.height))}:size;}
    if(part.kind==='genmoji')return {width:lineHeight,height:lineHeight};
    const attachment=item.attachments?.find(value=>value.id===part.attachment_id),mime=text(attachment?.data.mime_type);
    if(mime.startsWith('audio/'))return {width:Math.min(maxWidth,280),height:76+(text(fields(item.data.audio).transcription).trim()?44:0)};
    if(attachment&&!/^(image|video)\//u.test(mime))return {width:Math.min(maxWidth,260),height:88};
    if(attachment){
        const dimensions=fields(attachment.data.media),size=mediaSize(dimensions,maxWidth,prefs,attachmentHasAlpha(attachment));
        return mime.startsWith('video/')||dimensions.live===true?{width:Math.min(maxWidth,Math.max(120,size.width)),height:Math.min(mediaHeightCap(prefs),Math.max(120,size.height))}:size;
    }
    const card=messageCardData(item);
    if(card)return {width:Math.min(maxWidth,280),height:card.hasImage?180:88};
    return {width:Math.min(maxWidth,260),height:88};
}
export function commandPartIndex(item:ThreadItem){const index=item.data.command_part_index;return typeof index==='number'&&Number.isSafeInteger(index)&&index>=0?index:undefined;}
export function commandRenderKey(item:ThreadItem){const id=text(item.data.command_id),index=commandPartIndex(item);return id?'command:'+id+(index===undefined?'':':part:'+index):item.id;}
const presentationTime=(item:ThreadItem)=>text(item.data.command_sent_at)||item.data.sent_at;
function sameRun(a: ThreadItem|undefined,b: ThreadItem|undefined) { return !!a&&!!b&&a.data.kind!=='group_event'&&b.data.kind!=='group_event'&&!array(a.data.retracted_parts).length&&!array(b.data.retracted_parts).length&&Boolean(a.data.is_from_me)===Boolean(b.data.is_from_me)&&(a.data.is_from_me||sender(a)===sender(b))&&Math.abs(Date.parse(presentationTime(b))-Date.parse(presentationTime(a)))<=60000; }
type CachedRow={
    item:ThreadItem;
    previous:ThreadItem|undefined;
    next:ThreadItem|undefined;
    lastSent:boolean;
    latest:boolean;
    actor:string;
    target:string;
    initials:string|undefined;
    expanded:boolean;
    row:TranscriptRow;
};
type LayoutCache=Map<string,CachedRow>;
type LayoutRange={start:number;end:number};
export function createTranscriptLayout(){
    const cache:LayoutCache=new Map();
    let signature='';
    return (items:readonly ThreadItem[],handles:Handles,prefs:LayoutPrefs,width:number,measurementKey:string,range?:LayoutRange)=>{
        const now=prefs.now;
        const key=JSON.stringify([
            width,measurementKey,prefs.phone,prefs.group,prefs.service,
            prefs.viewportHeight,prefs.reduceMotion,prefs.scale,prefs.locale,
            now.getFullYear(),now.getMonth(),now.getDate()
        ]);
        if(key!==signature){
            cache.clear();
            signature=key;
        }
        if(!range||range.start===0){
            const present=new Set(items.map(item=>item.id));
            for(const id of cache.keys())if(!present.has(id))cache.delete(id);
        }
        return layout(items,handles,prefs,width,cache,range);
    };
}
export function layout(items: readonly ThreadItem[], handles: Handles, prefs: LayoutPrefs, width: number, cache?:LayoutCache,range?:LayoutRange): TranscriptRow[] {
    const fontSize=(prefs.phone?17:13)*prefs.scale,lineHeight=(prefs.phone?22:16)*prefs.scale;
    const maxWidth=Math.max(60,Math.min(prefs.phone?width*.7:480,width-(prefs.group?80:40))), padding=24;
    const lastSent=items.findLastIndex(item=>Boolean(item.data.is_from_me)&&item.data.kind!=='group_event'&&!item.data.retracted_at&&messageParts(item).some(part=>!array(item.data.retracted_parts).includes(part.index)));
    const source=range?items.slice(range.start,range.end):items;
    const rows=source.map((item,offset)=>{
        const index=offset+(range?.start??0);
        const data=item.data,sent=Boolean(data.is_from_me),event=fields(data.group_event);
        const actor=sent?wording.you:name(text(event.actor_handle_id)||sender(item),handles);
        const target=name(text(event.handle_id??event.target_handle_id),handles),initials=handles.get(sender(item))?.initials;
        const cached=cache?.get(item.id);
        const isLastSent=index===lastSent,isExpanded=prefs.expanded.has(item.id),latest=!isLastSent||prefs.atLatest!==false;
        if(cached&&cached.item===item&&cached.previous===items[index-1]&&cached.next===items[index+1]&&
            cached.lastSent===isLastSent&&cached.expanded===isExpanded&&cached.latest===latest&&
            cached.actor===actor&&cached.target===target&&cached.initials===initials)
            return cached.row;
        const previous=items[index-1],next=items[index+1],first=!sameRun(previous,item),last=!sameRun(item,next);
        const dateParts=!previous||Date.parse(presentationTime(item))-Date.parse(presentationTime(previous))>3600000?headerParts(presentationTime(item),prefs.now,prefs.locale):undefined,dateHeader=dateParts?`${dateParts.day}${dateParts.separator}${dateParts.time}`:undefined;
        const unsent=array(data.retracted_parts),parts=messageParts(item),visible=parts.filter(part=>!unsent.includes(part.index));
        const system=data.kind==='group_event'?groupEvent(data.group_event,actor,handles):data.retracted_at||visible.length===0?wording.unsent(actor):undefined;
        const bubbles: BubbleLayout[]=system?[]:visible.map((part,partIndex)=>{
            const emoji=part.kind==='text'&&emojiOnly(part.text??''),media=!['text','mention'].includes(part.kind);
            const content=text(part.text)||text(data.text),wrapped=wrapText(content,maxWidth-padding,prefs.measure);
            const size=partSize(item,part,maxWidth,lineHeight,prefs),subject=partIndex===0&&!media?text(data.subject):'',subjectSize=wrapText(subject,maxWidth-padding,value=>prefs.measure(value,{weight:700}));
            const bubbleWidth=media?size.width:emoji?Math.min(maxWidth,Math.ceil(prefs.measure(content)*2.7)+4):Math.min(maxWidth,Math.max(36,textWidthWithSlack(Math.max(wrapped.width,subject?subjectSize.width:0))+padding));
            const bubbleHeight=media?size.height:emoji?Math.ceil(fontSize*3):wrapped.lines*lineHeight+14+(subject?subjectSize.lines*lineHeight:0);
            const attachment=item.attachments?.find(value=>value.id===part.attachment_id),floating=part.kind==='genmoji'||Boolean(attachment&&attachmentHasAlpha(attachment));
            return {part,pendingAttachment:pendingAttachment(item),floating:floating||pendingAttachment(item)?.alpha===true,subject:subject||undefined,width:bubbleWidth,height:bubbleHeight,emoji,media,reactions:(item.reactions??[]).filter(reaction=>Number(reaction.data.part_index??0)===part.index),tail:last&&partIndex===visible.length-1&&!emoji};
        });
        const edited=Boolean(data.edited_at)||array(data.edit_history).length>0,history=prefs.expanded.has(item.id)?array(data.edit_history).map(value=>{const content=text(fields(value).text);const wrapped=wrapText(content,maxWidth-padding,prefs.measure);return {text:content,width:Math.min(maxWidth,Math.max(36,textWidthWithSlack(wrapped.width)+padding)),height:wrapped.lines*lineHeight+14};}):[];
        const receipt=index===lastSent&&prefs.atLatest!==false&&Number(data.error_code??0)===0?(data.read_at?`Read ${time(text(data.read_at),prefs.locale)}`:data.delivered_at?fields(data.delivery).quietly?wording.deliveredQuietly:wording.delivered:undefined):undefined;
        const reply=text(data.thread_originator_id),failed=Number(data.error_code??0)!==0;
        const commandID=text(data.command_id),commandState=failed?'failed':text(data.send_state)||'sent';
        const commandStatus=commandID&&(index===lastSent||commandState==='failed'||commandState==='indeterminate')?{id:commandID,state:commandState,label:commandState==='failed'?wording.notDelivered:commandState==='indeterminate'?wording.deliveryUnknown:commandState==='pending'?wording.sending:receipt??wording.sent}:undefined;
        const label=prefs.reduceMotion?effectLabel(data.effect):undefined;
        const gap=first?transcriptSpacing.betweenRuns:transcriptSpacing.withinRun,senderName=prefs.group&&!sent&&first&&!system?actor:undefined;
        let height=gap+(dateHeader?transcriptSpacing.dateLine+transcriptSpacing.afterHeader:0)+(senderName?transcriptSpacing.senderLine+transcriptSpacing.senderNameGap:0)+(reply&&reply!==item.id?26:0);
        height+=system?30:bubbles.reduce((sum,bubble,index)=>sum+bubble.height+(bubble.reactions.length?transcriptSpacing.reactionTop:0)+(index?transcriptSpacing.partGap:0),0);
        height+=(label?effectLineHeight:0)+(edited?22:0)+(commandStatus?44:(receipt?20:0)+(failed?20:0))+(item.reply_count?24:0)+history.reduce((sum,version)=>sum+version.height+4,0)+(unsent.length&&visible.length?24:0);
        const row:TranscriptRow={key:commandRenderKey(item),effectLabel:label,service:text(data.service)||prefs.service||'iMessage',commandStatus,id:item.id,item,height:Math.ceil(height),bubbles,sent,sender:actor,senderName,avatar:prefs.group&&!sent&&last&&!system?handles.get(sender(item))?.initials??'?':undefined,header:dateHeader,headerParts:dateParts,system,receipt:commandStatus?undefined:receipt,edited,history,reply:reply&&reply!==item.id?reply:undefined,failed:!commandStatus&&failed,gap,unsent:unsent.length&&visible.length?wording.unsent(actor):undefined};
        cache?.set(item.id,{item,previous,next,lastSent:isLastSent,expanded:isExpanded,latest,actor,target,initials,row});
        return row;
    });
    return stackImageRows(rows,prefs,maxWidth);
}

function stackImageRows(rows:TranscriptRow[],prefs:LayoutPrefs,maxWidth:number):TranscriptRow[]{
    const targeted=new Set(rows.flatMap(row=>(row.item.attachments??[]).map(attachment=>text(fields(attachment.data.sticker).target_message_id)).filter(Boolean)));
    const attachmentFor=(row:TranscriptRow)=>{
        if(row.bubbles.length!==1)return;
        const bubble=row.bubbles[0];
        if(bubble.part.kind!=='attachment'||bubble.reactions.length||bubble.floating)return;
        if(row.system||row.edited||row.unsent||row.failed||row.reply||row.item.reply_count||row.commandStatus||text(row.item.data.effect)||text(row.item.data.command_id)||targeted.has(row.id))return;
        const attachment=row.item.attachments?.find(value=>value.id===bubble.part.attachment_id),dimensions=fields(attachment?.data.media);
        const nativePart=array(row.item.data.parts).map(fields).find(part=>part.attachment_id===attachment?.id),nativeIndex=nativePart?.index;
        if(typeof nativeIndex!=='number'||!Number.isSafeInteger(nativeIndex)||nativeIndex<0||nativeIndex!==bubble.part.index)return;
        if(!attachment||!text(attachment.data.mime_type).startsWith('image/')||dimensions.live===true)return;
        if(Number(dimensions.width)<=24||Number(dimensions.height)<=24)return;
        if(![dimensions.width,dimensions.height].every(value=>typeof value==='number'&&Number.isFinite(value)&&value>0))return;
        return attachment;
    };
    const result:TranscriptRow[]=[];
    for(let index=0;index<rows.length;){
        const first=rows[index],firstAttachment=attachmentFor(first),members=[first],attachments:BridgeRecord[]=firstAttachment?[firstAttachment]:[];
        if(firstAttachment)for(let next=index+1;next<Math.min(rows.length,index+4);next++){
            const row=rows[next],attachment=attachmentFor(row);
            if(!attachment||row.bubbles[0].part.index!==first.bubbles[0].part.index||row.id===prefs.selectedMessageID||row.service!==first.service||!sameRun(rows[next-1].item,row.item))break;
            members.push(row);attachments.push(attachment);
        }
        if(members.length<2){result.push(first);index++;continue;}
        const last=members.at(-1)!,columns=Math.min(2,members.length),lines=Math.ceil(members.length/columns),dimensions=attachments.map(value=>fields(value.data.media));
        const minWidth=Math.min(...dimensions.map(value=>Number(value.width))),minHeight=Math.min(...dimensions.map(value=>Number(value.height)));
        const width=Math.min(maxWidth,prefs.phone?minWidth:columns*minWidth+(columns-1)*2),height=Math.min(mediaHeightCap(prefs),360,prefs.phone?minHeight:lines*minHeight+(lines-1)*2);
        const bubble={...first.bubbles[0],width,height,mediaAttachments:attachments,tail:false};
        const headerHeight=first.height-first.bubbles[0].height-(first.receipt?20:0);
        result.push({...first,members:members.map(row=>row.item),bubbles:[bubble],height:Math.ceil(headerHeight+height+(last.receipt?20:0)),avatar:last.avatar,receipt:last.receipt});
        index+=members.length;
    }
    return result;
}
