import {createEffect,createSignal,For,onCleanup,onMount} from 'solid-js';
import {revealHistory} from '../../motion/transcript';
import type {TranscriptRow} from './layout';
export function EditHistory(props:{versions:TranscriptRow['history']}){
    const [saved,setSaved]=createSignal<TranscriptRow['history']>([]);
    let outer!:HTMLDivElement,inner!:HTMLDivElement,reveal:ReturnType<typeof revealHistory>|undefined;
    createEffect(()=>{const versions=props.versions;if(versions.length)setSaved(versions.map(version=>({...version})));reveal?.setOpen(Boolean(versions.length));});
    onMount(()=>{reveal=revealHistory(outer,inner,()=>{if(!props.versions.length)setSaved([]);});reveal.setOpen(Boolean(props.versions.length));onCleanup(()=>reveal?.dispose());});
    return <div ref={outer} class="edit-history" hidden={!saved().length}><div ref={inner} class="edit-history-content"><For each={saved()}>{version=><div class="edit-version" style={{width:version.width+'px'}}>{version.text}</div>}</For></div></div>;
}
