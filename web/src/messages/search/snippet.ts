import type { ThreadItem } from '../../data/types';
import { strings as s } from '../../app/strings';
import { fields, text } from '../store/types';
import { messageText } from '../store/message';
import { matchRanges } from './matches';
export function searchSnippet(message:ThreadItem,query:string){
    const balloon=fields(message.data.balloon),body=messageText(message);
    const candidates=[...new Set([body,text(fields(balloon.link).title),text(fields(balloon.app).caption),...message.attachments.map(attachment=>text(attachment.data.filename))].filter(Boolean))].map(value=>({value,ranges:matchRanges(value,query)}));
    const matching=candidates.filter(candidate=>candidate.ranges.length);
    return (matching.length?matching:candidates).slice(0,3).map(({value,ranges})=>{let start=Math.max(0,(ranges[0]?.start??0)-80),end=Math.min(value.length,Math.max(start+260,(ranges[0]?.end??0)+80));if(start&&/[\uDC00-\uDFFF]/u.test(value[start]))start--;if(end<value.length&&/[\uDC00-\uDFFF]/u.test(value[end]))end--;return (start?'…':'')+value.slice(start,end)+(end<value.length?'…':'');}).join(' · ')||s.message;
}
