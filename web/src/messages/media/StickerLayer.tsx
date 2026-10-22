import {For,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {StickerImage} from './StickerImage';
import {stickerPlacementData} from './stickers';
export function StickerLayer(props:{stickers:readonly BridgeRecord[];partIndex:number;width:number;height:number}) {
 return <div class="sticker-layer" style={{position:'absolute',inset:'0',width:props.width+'px',height:props.height+'px',overflow:'visible','pointer-events':'none','z-index':'2'}}><For each={props.stickers}>{attachment=>{const placement=()=>stickerPlacementData(attachment);return <Show when={(placement().partIndex??0)===props.partIndex}><span class="sticker-placement" data-placement={placement().placement} data-placement-units={placement().sourceUnits} style={{position:'absolute',top:'0',right:'0',width:'64px',height:'64px',transform:'none','transform-origin':'center'}}><StickerImage attachment={attachment} size={64}/></span></Show>;}}</For></div>;
}
