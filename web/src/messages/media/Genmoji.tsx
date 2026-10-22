import type {BridgeRecord} from '../../data/types';
import {StickerImage} from './StickerImage';
export function Genmoji(props:{attachment:BridgeRecord;description?:string;size?:number}) {
 const size=()=>props.size!==undefined&&Number.isFinite(props.size)&&props.size>0?props.size+'px':'1em';
 return <span class="genmoji" style={{display:'inline-block',position:'relative',width:size(),height:size(),'vertical-align':'-.12em','line-height':'0'}}><StickerImage attachment={props.attachment} description={props.description} size="100%"/><svg class="genmoji-missing" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="1.5"/><path d="m4 18 5-5 3 3 4-6 4 8"/></svg></span>;
}
