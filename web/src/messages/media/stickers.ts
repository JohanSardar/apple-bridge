import type {BridgeRecord} from '../../data/types';
import {fields} from '../store/types';
export type StickerPlacementData={placement:'approximate';sourceUnits:'unverified';partIndex?:number;size:64;raw:Partial<Record<'x'|'y'|'scale'|'rotation',number>>};
export function stickerPlacementData(attachment:BridgeRecord):StickerPlacementData {
 const source=fields(attachment.data.sticker),raw:StickerPlacementData['raw']={};
 for(const key of ['x','y','scale','rotation'] as const){const value=source[key];if(typeof value==='number'&&Number.isFinite(value))raw[key]=value;}
 const part=source.part_index;
 return {placement:'approximate',sourceUnits:'unverified',partIndex:typeof part==='number'&&Number.isSafeInteger(part)&&part>=0?part:undefined,size:64,raw};
}
