import type {BridgeRecord} from '../../data/types';
import {createSignal,For,onCleanup,onMount,Show} from 'solid-js';
import {Portal} from 'solid-js/web';
import {reducedMotion,springAnimation} from '../../motion/animate';
import {classicReactions,type MessageActionsController} from './reactionActions';
import {reactionEmoji} from './emoji';
import './actions.css';
export function MessageActions(props:{actions:MessageActionsController;onForward?:(event:Event,text:string,attachment?:BridgeRecord)=>void}){
    const actions=props.actions;
    function Picker(){
        const [search,setSearch]=createSignal(''),[more,setMore]=createSignal(false);
        let dialog!:HTMLDialogElement,searchField:HTMLInputElement|undefined;
        const value=actions.target()!,anchor=value.anchor;
        let lift:ReturnType<typeof springAnimation>|undefined,rule:{sheet:CSSStyleSheet;rule:CSSRule}|undefined;
        onMount(()=>{
            dialog.showModal();
            const position=()=>{const bubble=anchor.getBoundingClientRect(),rect=dialog.getBoundingClientRect();dialog.style.left=Math.max(12,Math.min(innerWidth-rect.width-12,bubble.right-rect.width))+'px';dialog.style.top=Math.max(12,Math.min(innerHeight-rect.height-12,bubble.top>rect.height+24?bubble.top-rect.height-12:bubble.bottom+12))+'px';};
            position();const resize=new ResizeObserver(position);resize.observe(dialog);let frame=0;
            const moved=()=>{cancelAnimationFrame(frame);frame=requestAnimationFrame(position);};window.addEventListener('resize',moved);window.addEventListener('scroll',moved,true);onCleanup(()=>{resize.disconnect();cancelAnimationFrame(frame);window.removeEventListener('resize',moved);window.removeEventListener('scroll',moved,true);});
            const bubble=anchor.getBoundingClientRect();
            dialog.querySelector<HTMLElement>('button')?.focus({preventScroll:true});
            anchor.classList.add('message-action-anchor');
            const sheet=Array.from(document.styleSheets).find(sheet=>sheet.href?.startsWith(location.origin+'/'));
            if(sheet){const {left:x,top:y,right:r,bottom:b}=bubble,radius=Math.min(18,bubble.width/2,bubble.height/2),hole=`M${x+radius} ${y}H${r-radius}Q${r} ${y} ${r} ${y+radius}V${b-radius}Q${r} ${b} ${r-radius} ${b}H${x+radius}Q${x} ${b} ${x} ${b-radius}V${y+radius}Q${x} ${y} ${x+radius} ${y}Z`;const index=sheet.insertRule(`.message-actions-dialog::backdrop{clip-path:path(evenodd,"M0 0H${innerWidth}V${innerHeight}H0Z ${hole}")}`,sheet.cssRules.length);rule={sheet,rule:sheet.cssRules[index]};}
            if(!reducedMotion())lift=springAnimation(scale=>{anchor.style.scale=String(scale);},1,1.03,{response:.3,damping:.8});
            else{const fade=dialog.animate([{opacity:0},{opacity:1}],{duration:150});onCleanup(()=>fade.cancel());}
        });
        onCleanup(()=>{lift?.cancel();anchor.classList.remove('message-action-anchor');anchor.style.scale='';if(rule){const index=Array.from(rule.sheet.cssRules).indexOf(rule.rule);if(index>=0)rule.sheet.deleteRule(index);}});
        const results=()=>reactionEmoji.filter(([symbol,label,tags])=>(symbol+' '+label+' '+tags).toLocaleLowerCase().includes(search().trim().toLocaleLowerCase()));
        const keydown=(event:KeyboardEvent)=>{
            event.stopPropagation();
            if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)||event.target instanceof HTMLInputElement)return;
            const buttons=Array.from(dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
            const index=buttons.indexOf(document.activeElement as HTMLButtonElement);
            if(index<0)return;
            event.preventDefault();buttons[event.key==='Home'?0:event.key==='End'?buttons.length-1:(index+(event.key==='ArrowLeft'?-1:1)+buttons.length)%buttons.length]?.focus();
        };
        return <Portal><dialog ref={dialog} class="message-actions-dialog glass" aria-label="Message actions" data-message-version={value.item.version} onCancel={event=>{event.preventDefault();event.stopPropagation();actions.close();}} onKeyDown={keydown} onClick={event=>{if(event.target!==dialog)return;const rect=dialog.getBoundingClientRect();if(event.clientX<rect.left||event.clientX>rect.right||event.clientY<rect.top||event.clientY>rect.bottom)actions.close();}}>
            <Show when={actions.available()}><div class="tapback-picker" role="group" aria-label="Tapbacks"><For each={classicReactions}>{(reaction,index)=><button style={{'--reaction-index':index()}} aria-label={reaction.label} aria-pressed={actions.selected(reaction)} disabled={actions.busy()||actions.unknown()} onClick={event=>actions.react(event,reaction)}><span aria-hidden="true">{reaction.symbol}</span></button>}</For></div>
                <button class="message-action-item" disabled={actions.busy()||actions.unknown()} aria-expanded={more()} onClick={()=>{setMore(value=>!value);queueMicrotask(()=>searchField?.focus());}}>More reactions</button>
                <Show when={more()}><div class="reaction-search"><input ref={searchField} type="search" aria-label="Search reactions" value={search()} onInput={event=>setSearch(event.currentTarget.value)} placeholder="Search emoji"/><div class="reaction-emoji-grid"><For each={results()}>{([symbol,label])=><button aria-label={label} aria-pressed={actions.selected({kind:'emoji',emoji:symbol,symbol})} disabled={actions.busy()||actions.unknown()} onClick={event=>actions.react(event,{kind:'emoji',emoji:symbol,symbol})}>{symbol}</button>}</For></div><Show when={!results().length}><p class="action-problem">No emoji found</p></Show></div></Show>
            </Show>
            <Show when={actions.problem()}><p class="action-problem" role="status">{actions.problem()}</p></Show>
            <Show when={actions.unknown()}><button class="message-action-item" disabled={!actions.connected()} onClick={actions.check}>Check</button></Show>
            <Show when={actions.canReply()}><button class="message-action-item" onClick={actions.beginReply}>Reply</button></Show>
            <Show when={actions.changes.canEdit()}><button class="message-action-item" onClick={actions.changes.beginEdit}>Edit</button></Show>
            <Show when={actions.changes.canUnsend()}><button class="message-action-item" onClick={actions.changes.unsend}>Undo Send</button></Show>
            <Show when={actions.changes.pendingEdit()}><button class="message-action-item" onClick={actions.changes.copyPendingEdit}>Copy edit</button></Show>
            <Show when={actions.text()}><button class="message-action-item" onClick={actions.copy}>Copy</button></Show>
            <Show when={props.onForward&&actions.canForward()}><button class="message-action-item" onClick={event=>{if(!event.isTrusted||!actions.canForward())return;const text=actions.text();props.onForward?.(event,text,actions.forwardAttachment());}}>Forward</button></Show>
            <Show when={actions.canSave()}><button class="message-action-item" onClick={actions.save}>Save Attachment</button></Show>
            <Show when={actions.management.canDelete()}><button class="message-action-item destructive" onClick={actions.management.requestDelete}>Delete</button></Show>
            <button class="message-action-item" onClick={actions.close}>Close</button>
        </dialog></Portal>;
    }
    function DeleteConfirmation(){
        let dialog!:HTMLDialogElement;
        onMount(()=>{dialog.showModal();dialog.querySelector<HTMLButtonElement>('button')?.focus();});
        return <Portal><dialog ref={dialog} class="message-delete-dialog glass" aria-label="Delete Message" onCancel={event=>{event.preventDefault();event.stopPropagation();actions.management.cancelDelete();}} onKeyDown={event=>event.stopPropagation()}><h2>Delete Message?</h2><p>This message will be deleted for you.</p><Show when={actions.management.problem()}><p role="status">{actions.management.problem()}</p></Show><div><button class="button subtle" onClick={actions.management.cancelDelete}>Cancel</button><button class="button destructive" disabled={!actions.management.canConfirm()} onClick={actions.management.confirmDelete}>Delete</button></div></dialog></Portal>;
    }
    function Editor(){
        const changes=actions.changes,target=changes.editor()!.target;
        let field!:HTMLTextAreaElement,form!:HTMLFormElement;
        onMount(()=>{
            target.anchor.classList.add('message-editing');queueMicrotask(()=>{if(field.isConnected){field.focus({preventScroll:true});field.setSelectionRange(field.value.length,field.value.length);}});
            let frame=0;
            const reveal=()=>{cancelAnimationFrame(frame);frame=requestAnimationFrame(()=>{if(form.isConnected)form.scrollIntoView({block:'nearest',inline:'nearest'});});};
            const size=new ResizeObserver(reveal);size.observe(form);visualViewport?.addEventListener('resize',reveal);reveal();
            const entrance=form.animate([{opacity:0},{opacity:1}],{duration:150});
            onCleanup(()=>{entrance.cancel();cancelAnimationFrame(frame);size.disconnect();visualViewport?.removeEventListener('resize',reveal);target.anchor.classList.remove('message-editing');});
        });
        const status=()=>changes.editProblem()||(changes.editBusy()?['observed','delivered'].includes(changes.editState()?.status??'')?'Waiting for the updated message':'Saving…':'');
        return <Portal mount={target.anchor} ref={element=>element.classList.add('message-editor-mount')}><form ref={form} class="message-inline-editor" aria-label="Edit message" onSubmit={event=>{event.preventDefault();}} onKeyDown={event=>{event.stopPropagation();if(event.isComposing||event.keyCode===229)return;if(event.key==='Escape'){event.preventDefault();changes.cancelEdit();}else if(event.key==='Enter'&&!event.shiftKey){event.preventDefault();if(!event.repeat)changes.saveEdit(event);}}}>
            <textarea ref={field} aria-label="Edit message" value={changes.editor()?.text??''} disabled={changes.editBusy()} rows={3} onInput={event=>changes.setEditText(event.currentTarget.value)}/>
            <div class="message-edit-controls"><Show when={status()}><span role="status">{status()}</span></Show><Show when={changes.editLength()>65536}><span role="alert">Message is too long</span></Show><Show when={changes.editProblem()&&changes.editBusy()}><button type="button" disabled={!actions.connected()} onClick={()=>{const id=changes.editor()?.commandID;if(id)changes.check(id);}}>Check</button><button type="button" onClick={changes.copyEdit}>Copy</button></Show><button type="button" aria-label="Cancel edit" disabled={changes.editBusy()} onClick={changes.cancelEdit}>✕</button><button type="button" aria-label="Save edit" disabled={!changes.validEdit()} onClick={changes.saveEdit}>✓</button></div>
        </form></Portal>;
    }
    function Dissolve(){
        const value=actions.changes.dissolve()!,particles:HTMLSpanElement[]=[];
        onMount(()=>{
            const rect=value.target.anchor.getBoundingClientRect(),reduce=reducedMotion();
            const animation=value.target.anchor.animate(reduce?[{opacity:1},{opacity:0}]:[{opacity:1,filter:'blur(0px)',transform:'scale(1)'},{opacity:0,filter:'blur(6px)',transform:'translateY(-8px) scale(.96)'}],{duration:reduce?180:500,easing:'ease-out',fill:'forwards'});
            const burst=particles.map((particle,index)=>{particle.style.left=rect.left+(index+.5)*rect.width/particles.length+'px';particle.style.top=rect.top+rect.height/2+'px';return particle.animate([{opacity:0,transform:'translate(0,0) scale(1)'},{opacity:.8,offset:.25},{opacity:0,transform:`translate(${(index%3-1)*12}px,${-24-index%4*9}px) scale(0)`}],{duration:500,easing:'ease-out'});});
            Promise.all([animation,...burst].map(animation=>animation.finished)).catch(()=>{});
            onCleanup(()=>{animation.cancel();burst.forEach(animation=>animation.cancel());});
        });
        return <Portal><Show when={!reducedMotion()}><For each={[0,1,2,3,4,5,6,7,8,9,10,11]}>{()=> <span ref={element=>particles.push(element)} class="unsend-particle" aria-hidden="true"/>}</For></Show></Portal>;
    }
    function Flight(){
        let element!:HTMLSpanElement;
        const particles:HTMLSpanElement[]=[];
        const value=actions.flight()!;
        onMount(()=>{
            const from={x:value.from.left+value.from.width/2,y:value.from.top+value.from.height/2},to={x:value.to.right-12,y:value.to.top};
            element.style.left=from.x+'px';element.style.top=from.y+'px';
            const animation=element.animate(reducedMotion()?[{opacity:1},{opacity:0}]:[{transform:'translate(-50%,-50%) scale(1)',opacity:1},{transform:`translate(${(to.x-from.x)*.5-14}px,${Math.min(-48,(to.y-from.y)*.5-36)}px) scale(1.25)`,opacity:1,offset:.5},{transform:`translate(${to.x-from.x-14}px,${to.y-from.y-14}px) scale(.7)`,opacity:0}],{duration:reducedMotion()?150:420,easing:'cubic-bezier(.2,.7,.3,1)'});
            const burst=particles.map((particle,index)=>{particle.style.left=to.x+'px';particle.style.top=to.y+'px';return particle.animate([{transform:'translate(0,0) scale(1)',opacity:1},{transform:`translate(${Math.cos(index*Math.PI/3)*24}px,${Math.sin(index*Math.PI/3)*24}px) scale(0)`,opacity:0}],{delay:330,duration:260,easing:'ease-out',fill:'both'});});
            Promise.all([animation,...burst].map(value=>value.finished)).then(()=>{if(actions.flight()?.key===value.key)actions.clearFlight();}).catch(()=>{});
            onCleanup(()=>{animation.cancel();burst.forEach(value=>value.cancel());});
        });
        return <Portal><span ref={element} class="reaction-flight" aria-hidden="true">{value.symbol}</span><Show when={!reducedMotion()}><For each={[0,1,2,3,4,5]}>{()=> <span ref={element=>particles.push(element)} class="reaction-burst" style={{background:value.color}} aria-hidden="true"/>}</For></Show></Portal>;
    }
    return <><Show when={actions.target()} keyed><Picker/></Show><Show when={actions.management.confirmation()}><DeleteConfirmation/></Show><Show when={actions.changes.editor()}><Editor/></Show><Show when={actions.changes.dissolve()?.id} keyed><Dissolve/></Show><Show when={actions.flight()?.key} keyed><Flight/></Show></>;
}
