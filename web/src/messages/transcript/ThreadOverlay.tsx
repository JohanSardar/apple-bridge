import {createEffect,onCleanup,onMount,untrack,type Accessor,type JSX} from 'solid-js';
import {Portal} from 'solid-js/web';
import {Icon} from '../../design/Icon';
import {enterPane} from '../../motion/animate';
import './thread-overlay.css';
export function ThreadOverlay(props:{children:JSX.Element;onClose:(restoreFocus?:boolean)=>void;replyingTo?:Accessor<string|undefined>}){
    let dialog!:HTMLDialogElement,card!:HTMLDivElement,heading!:HTMLHeadingElement;
    const close=(restoreFocus=true)=>{if(dialog.open)dialog.close();props.onClose(restoreFocus);};
    let replyTarget=untrack(()=>props.replyingTo?.());
    createEffect(()=>{const target=props.replyingTo?.();if(target&&target!==replyTarget)close(false);replyTarget=target;});
    onCleanup(()=>{if(dialog?.open)dialog.close();});
    onMount(()=>{dialog.showModal();const cancel=enterPane(card,false,1,0);onCleanup(cancel);heading.focus({preventScroll:true});});
    return <Portal><dialog ref={dialog} class="thread-overlay" aria-label="Replies" onKeyDown={event=>event.stopPropagation()} onCancel={event=>{event.preventDefault();event.stopPropagation();close();}} onClick={event=>{if(event.target===event.currentTarget)close();}}><div ref={card} class="thread-card"><header><h2 ref={heading} tabindex="-1">Replies</h2><button class="thread-close glass" aria-label="Close replies" onClick={()=>close()}><Icon name="close"/></button></header><div class="thread-card-content">{props.children}</div></div></dialog></Portal>;
}
