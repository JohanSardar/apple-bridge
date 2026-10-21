import {createSignal,For,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {Icon} from '../../design/Icon';
import {name,wording} from '../format';
import {sender,text,type Handles} from '../store/types';
const symbols:Record<string,string>={love:'❤️',like:'👍',dislike:'👎',laugh:'😂',emphasize:'‼️',question:'❓'};
function symbol(reaction:BridgeRecord){const kind=text(reaction.data.kind);return kind==='emoji'?text(reaction.data.emoji)||'Reaction':Object.hasOwn(symbols,kind)?symbols[kind]:kind==='sticker'?'Sticker':'Reaction';}
export function ReactionBadges(props:{reactions:readonly BridgeRecord[];handles:Handles}){
    const [open,setOpen]=createSignal(false);
    const reactor=(reaction:BridgeRecord)=>reaction.data.is_from_me?wording.you:name(sender(reaction),props.handles);
    const content=(reaction:BridgeRecord)=><Show when={reaction.data.kind==='sticker'} fallback={symbol(reaction)}><span data-sticker-placeholder aria-hidden="true">▧</span></Show>;
    return <Show when={props.reactions.length}><div class="reaction-stack" aria-label="Message reactions">
        <For each={props.reactions.slice(0,4)}>{(reaction,index)=><button class="reaction-badge" classList={{'own-reaction':Boolean(reaction.data.is_from_me)}} style={{'--reaction-index':index()}} aria-label={`${reactor(reaction)} reacted ${symbol(reaction)}`} aria-haspopup="dialog" aria-expanded={open()} onClick={()=>setOpen(true)}>{content(reaction)}</button>}</For>
        <Show when={props.reactions.length>4}><button class="reaction-badge reaction-overflow" style={{'--reaction-index':4}} aria-label={`See all ${props.reactions.length} reactions`} aria-haspopup="dialog" onClick={()=>setOpen(true)}>+{props.reactions.length-4}</button></Show>
    </div><Show when={open()}><dialog class="reactions-dialog who-reacted" aria-label="Reactions" ref={element=>queueMicrotask(()=>{if(element.isConnected)element.showModal();})} onClose={()=>setOpen(false)} onCancel={event=>event.stopPropagation()} onKeyDown={event=>event.stopPropagation()} onClick={event=>{if(event.target!==event.currentTarget)return;const rect=event.currentTarget.getBoundingClientRect();if(event.clientX<rect.left||event.clientX>rect.right||event.clientY<rect.top||event.clientY>rect.bottom)event.currentTarget.close();}}>
        <header><h2>Reactions</h2><button class="icon-button" aria-label="Close Reactions" onClick={event=>event.currentTarget.closest('dialog')!.close()}><Icon name="close"/></button></header>
        <ul><For each={props.reactions}>{reaction=><li><span class="reactor-avatar" aria-hidden="true">{reaction.data.is_from_me?wording.you.slice(0,1):props.handles.get(sender(reaction))?.initials??'?'}</span><span class="reactor-name">{reactor(reaction)}</span><span class="reaction-symbol" aria-label={symbol(reaction)}>{content(reaction)}</span></li>}</For></ul>
    </dialog></Show></Show>;
}
