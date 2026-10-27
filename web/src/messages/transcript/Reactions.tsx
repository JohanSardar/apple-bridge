import {createEffect,createMemo,createSignal,createUniqueId,For,onCleanup,onMount,Show,untrack} from 'solid-js';
import {popReaction} from '../../motion/transcript';
import {Portal} from 'solid-js/web';
import {bubbleSilhouette} from './shape';
import type {BridgeRecord} from '../../data/types';
import {Icon} from '../../design/Icon';
import {name,wording} from '../format';
import {text,type Handles} from '../store/types';
import {sender} from '../store/message';
const symbols:Record<string,string>={love:'❤️',like:'👍',dislike:'👎',laugh:'😂',emphasize:'‼️',question:'❓'};
function symbol(reaction:BridgeRecord){const kind=text(reaction.data.kind);return kind==='emoji'?text(reaction.data.emoji)||'Reaction':Object.hasOwn(symbols,kind)?symbols[kind]:kind==='sticker'?'Sticker':'Reaction';}
function identity(reaction:BridgeRecord){const data=reaction.data;return [data.kind,data.kind==='emoji'?data.emoji:data.kind==='sticker'?data.sticker_id??data.attachment_id:''].join(':');}
export function ReactionBadges(props:{reactions:readonly BridgeRecord[];handles:Handles}){
    const [open,setOpen]=createSignal(false);
    const popoverID=createUniqueId();
    let anchor!:HTMLDivElement;
    let trigger:HTMLButtonElement|undefined,dialog:HTMLDialogElement|undefined,returnViewport:HTMLElement|undefined;
    let knownGroups:Map<string,number>|undefined,popping=new Set<string>();
    let raised:HTMLElement|undefined,backdropRule:{sheet:CSSStyleSheet;rule:CSSRule}|undefined;
    const restore=()=>{raised?.classList.remove('reaction-anchor-raised');raised=undefined;if(backdropRule){const index=Array.from(backdropRule.sheet.cssRules).indexOf(backdropRule.rule);if(index>=0)backdropRule.sheet.deleteRule(index);backdropRule=undefined;}};
    const close=()=>{const wasOpen=untrack(open);if(dialog?.open)dialog.close();restore();setOpen(false);if(wasOpen)queueMicrotask(()=>{const target=trigger?.isConnected?trigger:returnViewport;if(target?.isConnected)target.focus({preventScroll:true});});};
    onCleanup(restore);
    const groups=createMemo(()=>{const result=new Map<string,BridgeRecord[]>();for(const reaction of [...props.reactions].sort((a,b)=>Date.parse(text(b.data.sent_at))-Date.parse(text(a.data.sent_at))||b.id.localeCompare(a.id))){const key=identity(reaction),group=result.get(key);if(group)group.push(reaction);else result.set(key,[reaction]);}const next=[...result.values()];popping=new Set(next.filter(group=>knownGroups!==undefined&&group.length>(knownGroups.get(identity(group[0]))??0)).map(group=>identity(group[0])));knownGroups=new Map(next.map(group=>[identity(group[0]),group.length]));return next;});
    createEffect(()=>{if(!groups().length&&untrack(open))close();});
    const reactor=(reaction:BridgeRecord)=>reaction.data.is_from_me?wording.you:name(sender(reaction),props.handles);
    const content=(reaction:BridgeRecord)=><Show when={reaction.data.kind==='sticker'} fallback={symbol(reaction)}><span data-sticker-placeholder aria-hidden="true">▧</span></Show>;
    const position=(element:HTMLDialogElement)=>{dialog=element;queueMicrotask(()=>{
        if(!element.isConnected)return;
        element.showModal();element.querySelector<HTMLElement>('h2')?.focus({preventScroll:true});
        if(!matchMedia('(min-width:700px)').matches)return;
        raised=anchor.parentElement?.querySelector<HTMLElement>('.bubble')??undefined;
        if(!raised)return;
        const bubble=raised.getBoundingClientRect(),badge=(trigger??anchor).getBoundingClientRect(),below=innerHeight-bubble.bottom-24,above=bubble.top-24;
        const bottom=below>=element.getBoundingClientRect().height||below>=above,available=Math.max(72,Math.min(420,innerHeight*.7,bottom?below:above));
        element.style.setProperty('--popover-height',available+'px');
        const height=element.getBoundingClientRect().height,center=(badge.left+badge.right)/2,left=Math.max(12,Math.min(innerWidth-332,center-32));
        element.style.left=left+'px';element.style.top=(bottom?bubble.bottom+12:bubble.top-height-12)+'px';
        element.dataset.side=bottom?'below':'above';element.style.setProperty('--arrow-left',Math.max(18,Math.min(302,center-left))+'px');
        const silhouette=bubbleSilhouette(bubble.width,bubble.height,{x:bubble.left,y:bubble.top,mirror:!raised.classList.contains('sent'),tail:raised.classList.contains('has-tail'),radius:raised.classList.contains('floating-media')?0:18});
        const sheet=Array.from(document.styleSheets).find(value=>value.href?.startsWith(location.origin+'/'));
        if(sheet){const index=sheet.insertRule(`[data-reaction-popover="${popoverID}"]::backdrop{clip-path:path(evenodd,"M0 0H${innerWidth}V${innerHeight}H0Z ${silhouette}")}`,sheet.cssRules.length);backdropRule={sheet,rule:sheet.cssRules[index]};}
        raised.classList.add('reaction-anchor-raised');
    });};
    return <Show when={groups().length}><div ref={anchor} class="reaction-stack" aria-label="Message reactions">
        <For each={groups().slice(0,3)}>{(group,index)=>{let button!:HTMLButtonElement;onMount(()=>{if(popping.has(identity(group[0])))onCleanup(popReaction(button));});return <button ref={button} data-reaction-key={identity(group[0])} class="reaction-badge" classList={{'own-reaction':group.some(reaction=>Boolean(reaction.data.is_from_me))}} style={{'--reaction-index':index()}} aria-label={`${group.length===1?reactor(group[0]):group.length+' people'} reacted ${symbol(group[0])}`} aria-haspopup="dialog" aria-expanded={open()} onClick={event=>{trigger=event.currentTarget;returnViewport=anchor.closest<HTMLElement>('.virtual-viewport')??undefined;setOpen(true);}}>{content(group[0])}<Show when={group.length>1}><span class="reaction-count">{group.length}</span></Show></button>;}}</For>
        <Show when={groups().length>3}><button class="reaction-badge reaction-overflow" style={{'--reaction-index':3}} aria-label={`See all ${groups().length} reactions`} aria-haspopup="dialog" onClick={event=>{trigger=event.currentTarget;returnViewport=anchor.closest<HTMLElement>('.virtual-viewport')??undefined;setOpen(true);}}>+{groups().length-3}</button></Show>
    </div><Show when={open()}><Portal><dialog class="reactions-dialog who-reacted" data-reaction-popover={popoverID} aria-label="Reactions" ref={position} onClose={close} onCancel={event=>{event.preventDefault();event.stopPropagation();close();}} onKeyDown={event=>event.stopPropagation()} onClick={event=>{if(event.target!==event.currentTarget)return;const rect=event.currentTarget.getBoundingClientRect();if(event.clientX<rect.left||event.clientX>rect.right||event.clientY<rect.top||event.clientY>rect.bottom)close();}}>
        <span class="reaction-arrow" data-reaction-arrow aria-hidden="true"/>
        <header><h2 tabindex="-1">Reactions</h2><button class="reaction-close glass" aria-label="Close Reactions" onClick={close}><Icon name="close"/></button></header>
        <ul><For each={props.reactions}>{reaction=><li><span class="reactor-avatar" aria-hidden="true">{reaction.data.is_from_me?wording.you.slice(0,1):props.handles.get(sender(reaction))?.initials??'?'}</span><span class="reactor-name">{reactor(reaction)}</span><span class="reaction-symbol" aria-label={symbol(reaction)}>{content(reaction)}</span></li>}</For></ul>
    </dialog></Portal></Show></Show>;
}
