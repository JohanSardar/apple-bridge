import type {BridgeRecord,ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
export function messageAudioData(message:ThreadItem,attachment:BridgeRecord){
 const metadata=fields(attachment.data.media),flags=fields(message.data.audio),samples=Array.isArray(metadata.waveform)?metadata.waveform:[],waveform=samples.filter((value):value is number=>typeof value==='number'&&Number.isFinite(value)).slice(0,256),duration=Number(metadata.duration_ms);
 return {waveform:waveform.length?Array.from({length:Math.min(64,waveform.length)},(_,index)=>{const start=Math.floor(index*waveform.length/Math.min(64,waveform.length)),end=Math.floor((index+1)*waveform.length/Math.min(64,waveform.length));return Math.min(1,Math.max(0,...waveform.slice(start,end).map(value=>value/255)));}):[],duration:Number.isFinite(duration)&&duration>0?duration/1000:0,transcription:text(flags.transcription),played:Boolean(flags.played_at),expirable:flags.expirable===true,kept:flags.kept===true};
}
