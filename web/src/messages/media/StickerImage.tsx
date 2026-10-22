import {createSignal,onCleanup,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {fields,text} from '../store/types';
import {MediaImage} from './MediaImage';
import {StickerMotion} from './StickerMotion';
import './stickers.css';
export function StickerImage(props:{attachment:BridgeRecord;size:number|'100%';description?:string}) {
 const preference=matchMedia('(prefers-reduced-motion:reduce)'),[reduced,setReduced]=createSignal(preference.matches);
 const change=()=>setReduced(preference.matches);preference.addEventListener('change',change);onCleanup(()=>preference.removeEventListener('change',change));
 const animatedImage=()=>!reduced()&&/^image\/(gif|webp)$/u.test(text(props.attachment.data.mime_type));
 const still=()=>fields(props.attachment.data.media).live===true&&text(fields(fields(props.attachment.data.derivatives).poster).digest)?'poster':props.attachment.data.is_sticker===true?'display':'thumb';
 const size=()=>typeof props.size==='number'?(Number.isFinite(props.size)&&props.size>0?props.size:64)+'px':props.size;
 return <span class="sticker-image" style={{width:size(),height:size()}}><MediaImage attachment={props.attachment} variant={animatedImage()?'original':still()} alt={props.description??(text(props.attachment.data.description)||'Sticker')} interactive={false}/><Show when={!reduced()&&fields(props.attachment.data.media).live===true}><StickerMotion attachment={props.attachment}/></Show></span>;
}
