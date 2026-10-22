import type {TextEffect} from './data';
export type TextStyle='bold'|'italic'|'underline'|'strikethrough';
export type FormattingSegment={start:number;end:number;text:string;styles:TextStyle[];effect?:TextEffect};
const styles:readonly TextStyle[]=['bold','italic','underline','strikethrough'];
const effects:readonly string[]=['big','small','shake','nod','explode','ripple','bloom','jitter'];
export function formattingSegments(text:string,ranges:readonly unknown[]=[]):FormattingSegment[] {
 const boundary=(index:number)=>index<=0||index>=text.length||!(text.charCodeAt(index-1)>=0xd800&&text.charCodeAt(index-1)<=0xdbff&&text.charCodeAt(index)>=0xdc00&&text.charCodeAt(index)<=0xdfff);
 const valid=ranges.flatMap(raw=>{if(!raw||typeof raw!=='object')return [];const range=raw as Record<string,unknown>,start=range.utf16_location,length=range.utf16_length;if(typeof start!=='number'||typeof length!=='number'||!Number.isSafeInteger(start)||!Number.isSafeInteger(length)||start<0||length<=0||start>=text.length)return [];const end=Math.min(text.length,start+length);if(!Number.isSafeInteger(end)||!boundary(start)||!boundary(end))return [];const requested=Array.isArray(range.styles)?range.styles:[],effect=typeof range.effect==='string'&&effects.includes(range.effect)?range.effect as TextEffect:undefined;return [{start,end,styles:styles.filter(style=>requested.includes(style)),effect}];});
 const points=[...new Set([0,text.length,...valid.flatMap(range=>[range.start,range.end])])].sort((a,b)=>a-b),segments:FormattingSegment[]=[];
 for(let index=1;index<points.length;index++){const start=points[index-1],end=points[index];if(start===end)continue;const active=valid.filter(range=>range.start<=start&&range.end>=end),segment:FormattingSegment={start,end,text:text.slice(start,end),styles:styles.filter(style=>active.some(range=>range.styles.includes(style))),effect:active.findLast(range=>range.effect)?.effect},previous=segments.at(-1);if(previous&&previous.effect===segment.effect&&previous.styles.join(',')===segment.styles.join(',')){previous.end=end;previous.text+=segment.text;}else segments.push(segment);}
 return segments;
}
