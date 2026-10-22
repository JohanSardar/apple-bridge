import { createEffect, createMemo, createSignal, lazy, onCleanup, onMount, Show, Suspense, untrack } from 'solid-js';
import { Icon } from '../design/Icon';
import { strings as s } from './strings';
import { router } from './router';
import { AppMenu } from './AppMenu';
import { registerShortcut } from './keyboard';
import { appearance } from './preferences';
import { auth } from '../auth/session';
import { createMessageCommands } from '../messages/actions/commands';
import { createMessageActions } from '../messages/actions/reactionActions';
import { ConversationList } from '../messages/list/ConversationList';
import { enterPane, reducedMotion, crossfade, springAnimation } from '../motion/animate';
import { drag, swipeBack } from '../motion/gestures';
const NewMessage=lazy(()=>import('../messages/compose/NewMessage'));
const MessagePane=lazy(()=>import('../messages/MessagePane'));
export default function Shell() {
    const messageCommands=createMessageCommands(auth.live()!);
    const messageActions=createMessageActions(auth.live()!,messageCommands.capabilities);
    const [details, setDetails] = createSignal(false), [inspectorVisible, setInspectorVisible] = createSignal(false);
    const phoneMedia=matchMedia('(max-width:699px)');
    const [phone,setPhone]=createSignal(phoneMedia.matches);
    const compactMedia = matchMedia('(max-width:1023px)');
    const [compact, setCompact] = createSignal(compactMedia.matches);
    let transcript!: HTMLElement, sidebar!: HTMLElement, resizer!: HTMLDivElement, inspector: HTMLElement | undefined;
    let cancelEnter: (() => void) | undefined, inspectorAnimation: ReturnType<typeof springAnimation> | undefined;
    const chat = createMemo(() => router.route().chat);
    const [threadTitle,setThreadTitle]=createSignal(''),[paneWidth,setPaneWidth]=createSignal(0);
    let measuredPhone=phoneMedia.matches;
    const measuredPaneWidth=()=>untrack(paneWidth)||paneWidth();
    const threadRoute=createMemo(()=>{
        const path=new URL(router.route().path,location.origin).pathname.split('/').filter(Boolean);
        const decode=(value?:string)=>{try{return decodeURIComponent(value??'');}catch{return value??'';}};
        const id=decode(path[1]);
        return path[0]==='messages'&&id&&!['new','search'].includes(id)?{chatID:id,messageID:path[2]&&path[2]!=='details'?decode(path[2]):undefined}:undefined;
    });
    let previousChat:string|undefined;
    createEffect(()=>{const id=threadRoute()?.chatID;if(id!==previousChat){previousChat=id;setThreadTitle('');}});
    const hasConversation=()=>Boolean(chat()&&!['new','search'].includes(chat()!));
    const phoneDetails=()=>phone()&&router.route().panel==='details';
    let inspectorFrame=0,inspectorFade:Animation|undefined;
    function openInspector(open: boolean) {
        if(open&&!hasConversation())return;
        if(phone()){if(open&&chat())router.navigate('/messages/'+encodeURIComponent(chat()!)+'/details');else if(!open&&phoneDetails())router.back();return;}
        if(!open&&router.route().panel==='details')router.navigate('/messages/'+encodeURIComponent(chat()!),true);
        const visible=inspectorVisible();
        setDetails(open);
        cancelAnimationFrame(inspectorFrame);
        inspectorFade?.cancel();
        if(compact()){inspectorAnimation?.cancel();setInspectorVisible(false);return;}
        if(open)setInspectorVisible(true);
        if(!inspector)return;
        if(reducedMotion()){
            inspectorFade=crossfade(inspector,open);
            if(!open)inspectorFade.finished.then(()=>{if(!details())setInspectorVisible(false);}).catch(()=>{});
            return;
        }
        if(visible&&inspectorAnimation){inspectorAnimation.retarget(open?0:320);return;}
        inspectorAnimation?.cancel();
        const direction=getComputedStyle(inspector).direction==='rtl'?-1:1;
        const from=open?320:new DOMMatrix(getComputedStyle(inspector).transform).m41*direction;
        inspector.style.transform=`translateX(${from*direction}px)`;
        inspectorFrame=requestAnimationFrame(()=>{
            inspectorAnimation=springAnimation(value=>{if(inspector)inspector.style.transform=`translateX(${value*direction}px)`;},from,open?0:320,{response:.35,damping:1},0,()=>{if(!details())setInspectorVisible(false);else if(inspector)inspector.style.transform='';});
        });
    }
    onMount(() => {
        const mediaChanged = () => { setCompact(compactMedia.matches); setInspectorVisible(details() && !compactMedia.matches); };
        compactMedia.addEventListener('change', mediaChanged);
        const phoneChanged=()=>{if(measuredPhone!==phoneMedia.matches)setPaneWidth(0);setPhone(phoneMedia.matches);};
        phoneMedia.addEventListener('change',phoneChanged);
        const measurements=new ResizeObserver(entries=>{measuredPhone=phoneMedia.matches;setPaneWidth(Math.round(entries[0].contentRect.width));});measurements.observe(transcript);onCleanup(()=>measurements.disconnect());
        if(router.route().panel==='details'&&!phone())openInspector(true);
        const dispose = swipeBack(transcript, sidebar, () => phone() && Boolean(chat()), () => router.back(),()=>cancelEnter?.());
        let width = 320;
        const resize = drag(resizer, { start: event => { event.preventDefault(); width = sidebar.getBoundingClientRect().width; return true; }, move: x => document.documentElement.style.setProperty('--sidebar-width', Math.max(280, Math.min(420, width + x * (getComputedStyle(sidebar).direction === 'rtl' ? -1 : 1))) + 'px'), end: x => { appearance.setSidebarWidth(width + x * (getComputedStyle(sidebar).direction === 'rtl' ? -1 : 1)); } });
        const shortcuts = [registerShortcut({ key: 'n', input:true, action: () => router.navigate('/messages/new') }), registerShortcut({ key: ',', action: () => router.navigate('/settings/appearance') }), registerShortcut({ key: '[', action: () => router.back() }), registerShortcut({ key: 'i', action: () => openInspector(!details()) }), registerShortcut({ key: '=', action: () => appearance.setTextSize(Math.min(1.5, appearance.textSize() + .05)) }), registerShortcut({ key: '+', shift: true, action: () => appearance.setTextSize(Math.min(1.5, appearance.textSize() + .05)) }), registerShortcut({ key: '-', action: () => appearance.setTextSize(Math.max(.85, appearance.textSize() - .05)) }), registerShortcut({ key: '0', action: () => appearance.setTextSize(1) })];
        onCleanup(() => { dispose(); resize(); shortcuts.forEach(dispose => dispose()); cancelEnter?.(); inspectorAnimation?.cancel(); inspectorFade?.cancel(); cancelAnimationFrame(inspectorFrame); compactMedia.removeEventListener('change', mediaChanged); phoneMedia.removeEventListener('change',phoneChanged); });
    });
    createEffect(() => { const selected = chat(); if (selected && transcript) {
        const isPhone=phone(),width=isPhone?measuredPaneWidth():0;if(isPhone&&!width)return;
        cancelEnter?.();
        cancelEnter = enterPane(transcript, isPhone, getComputedStyle(transcript).direction === 'rtl' ? -1 : 1,width);
    } });
    function PhoneDetails(){
        let pane!:HTMLElement;
        let cancel:()=>void=()=>{};
        let dispose:()=>void=()=>{};
        onMount(()=>{createEffect(()=>{const width=measuredPaneWidth();if(!width)return;cancel();cancel=enterPane(pane,true,getComputedStyle(pane).direction==='rtl'?-1:1,width);});dispose=swipeBack(pane,transcript,()=>phoneDetails(),()=>router.back(),()=>cancel());});
        onCleanup(()=>{cancel();dispose();});
        return <section ref={pane} class="phone-details" role="main" aria-label={s.details}><header class="toolbar glass"><button class="icon-button" aria-label={s.back} onClick={()=>router.back()}><Icon name="chevron"/></button><h2>{s.details}</h2></header><DetailContent/></section>;
    }
    function DetailContent() { return <div class="empty quiet-empty"><p>{s.noDetails}</p></div>; }
    return <div class="app-shell" classList={{ 'has-chat': Boolean(chat()), 'inspector-open': inspectorVisible() }}><aside ref={sidebar} class="sidebar" role={phone()&&!chat()?'main':undefined} inert={phone()&&Boolean(chat())}><Show when={auth.live()}>{live=><ConversationList live={live()}/>}</Show><footer class="sidebar-footer"><AppMenu/></footer></aside><div ref={resizer} class="pane-resizer" role="separator" aria-label={s.resizeSidebar} aria-orientation="vertical" aria-valuemin="280" aria-valuemax="420" aria-valuenow={appearance.sidebarWidth()} tabindex="0" onKeyDown={event => { if (['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) {
        event.preventDefault();
        const direction=getComputedStyle(sidebar).direction==='rtl'?-1:1;
        const width=event.key==='Home'?280:event.key==='End'?420:sidebar.getBoundingClientRect().width+(event.key==='ArrowRight'?10:-10)*direction;
        appearance.setSidebarWidth(width);
    } }}/><main ref={transcript} inert={phoneDetails()} class="transcript" data-scroll="transcript"><header class="toolbar glass"><button class="icon-button back-button" aria-label={s.back} onClick={() => router.back()}><Icon name="chevron"/></button><span class="toolbar-title"><Show when={threadRoute()} fallback={chat()==='new'?s.compose:''}><span class="chat-avatar" aria-hidden="true">{threadTitle().split(/\s+/u).slice(0,2).map(value=>Array.from(value)[0]??'').join('')}</span><span>{threadTitle()}</span></Show></span><Show when={hasConversation()}><button class="icon-button" aria-label={s.details} aria-expanded={details()||router.route().panel==='details'} onClick={() => openInspector(!details())}><Icon name="info"/></button></Show></header><div class="transcript-body"><Show when={chat()==='new'} fallback={<Show when={threadRoute()?.chatID} keyed fallback={<div class="empty quiet-empty"><p>{s.noSelection}</p></div>}>{id=><Suspense><MessagePane chatID={id} messageID={threadRoute()?.messageID} title={setThreadTitle} actions={messageActions} live={auth.live()!} commands={messageCommands} session={auth.session()!}/></Suspense>}</Show>}><Suspense><NewMessage commands={messageCommands}/></Suspense></Show></div></main><Show when={inspectorVisible() && !compact()}><aside ref={inspector} class="inspector" aria-label={s.details}><header class="toolbar glass"><span class="toolbar-title">{s.details}</span><button class="icon-button" aria-label={s.closeDetails} onClick={() => openInspector(false)}><Icon name="close"/></button></header><DetailContent /></aside></Show><Show when={!phone()&&compact()&&(details()||router.route().panel==='details')}><dialog ref={element => queueMicrotask(() => element.showModal())} class="details-sheet glass" aria-label={s.details} onClose={() => openInspector(false)}><button autofocus class="icon-button sheet-close" aria-label={s.closeDetails} onClick={() => setDetails(false)}><Icon name="close"/></button><DetailContent /></dialog></Show><Show when={phoneDetails()}><PhoneDetails/></Show></div>;
}
