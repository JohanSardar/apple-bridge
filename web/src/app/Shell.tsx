import { createEffect, createMemo, createSignal, For, lazy, onCleanup, onMount, Show, Suspense, untrack } from 'solid-js';
import { Icon, type IconName } from '../design/Icon';
import { strings as s } from './strings';
import { router } from './router';
import { registerShortcut } from './keyboard';
import { appearance } from './preferences';
import { auth } from '../auth/session';
import { enterPane, reducedMotion, crossfade, springAnimation } from '../motion/animate';
import { drag, swipeBack } from '../motion/gestures';
const Transcript=lazy(()=>import('../messages/transcript/Transcript'));
const apps: [
    string,
    IconName
][] = [[s.messages, 'messages'], [s.reminders, 'reminders'], [s.calendar, 'calendar'], [s.contacts, 'contacts'], [s.notes, 'notes']];
export default function Shell() {
    const [switcher, setSwitcher] = createSignal(false), [filters, setFilters] = createSignal(false), [details, setDetails] = createSignal(false), [inspectorVisible, setInspectorVisible] = createSignal(false);
    const [filter, setFilter] = createSignal(s.allMessages as string);
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
    const phoneDetails=()=>phone()&&router.route().panel==='details';
    let inspectorFrame=0,inspectorFade:Animation|undefined;
    function openInspector(open: boolean) {
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
        const resize = drag(resizer, { start: () => { width = sidebar.getBoundingClientRect().width; return true; }, move: x => document.documentElement.style.setProperty('--sidebar-width', Math.max(280, Math.min(420, width + x * (getComputedStyle(sidebar).direction === 'rtl' ? -1 : 1))) + 'px'), end: () => { } });
        const shortcuts = [registerShortcut({ key: ',', action: () => router.navigate('/settings/appearance') }), registerShortcut({ key: '[', action: () => router.back() }), registerShortcut({ key: 'i', action: () => openInspector(!details()) }), registerShortcut({ key: 'n', action: () => router.navigate('/messages/new') }), registerShortcut({ key: '=', action: () => appearance.setTextSize(Math.min(1.5, appearance.textSize() + .05)) }), registerShortcut({ key: '+', shift: true, action: () => appearance.setTextSize(Math.min(1.5, appearance.textSize() + .05)) }), registerShortcut({ key: '-', action: () => appearance.setTextSize(Math.max(.85, appearance.textSize() - .05)) }), registerShortcut({ key: '0', action: () => appearance.setTextSize(1) })];
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
    return <div class="app-shell" classList={{ 'has-chat': Boolean(chat()), 'inspector-open': inspectorVisible() }}><aside ref={sidebar} class="sidebar" role={phone()&&!chat()?'main':undefined} inert={phone()&&Boolean(chat())}><header class="toolbar sidebar-toolbar glass"><div class="filter-control"><button class="icon-button" aria-label={s.filters} aria-expanded={filters()} onClick={() => setFilters(!filters())}><Icon name="ellipsis"/></button><Show when={filters()}><div class="filter-menu glass" role="group" aria-label={s.filters}><For each={[s.allMessages, s.knownSenders, s.unknownSenders, s.unread]}>{name => <button aria-pressed={filter() === name} onClick={() => { setFilter(name); setFilters(false); }}>{name}<Show when={filter() === name}><Icon name="check"/></Show></button>}</For><button disabled>{s.edit}</button></div></Show></div><label class="sidebar-search"><Icon name="search"/><input type="search" aria-label={s.search} placeholder={s.search}/></label><span class="phone-toolbar-spacer"/><button class="icon-button" aria-label={s.compose} onClick={() => router.navigate('/messages/new')}><Icon name="compose"/></button></header><div class="sidebar-heading"><h1>{s.messages}</h1></div><Show when={auth.live()?.status() === 'offline'}><p class="offline-banner" role="status">{s.offline}</p></Show><div class="sidebar-body" data-scroll="sidebar"><p class="no-messages">{s.noMessages}</p></div><footer class="sidebar-footer"><div class="app-selector"><button class="icon-button app-menu-control glass" aria-label={s.switchApp} aria-expanded={switcher()} onClick={() => setSwitcher(!switcher())}><Icon name="grid"/></button><Show when={switcher()}><div class="app-menu glass" role="group" aria-label={s.appMenu} onKeyDown={event => { if (event.key === 'Escape')
        setSwitcher(false); }}><For each={apps}>{([name, icon], index) => <button disabled={index() > 0} onClick={() => setSwitcher(false)}><span class={'app-icon app-icon-' + icon}><Icon name={icon}/></span><span>{name}<Show when={index() > 0}><small>{s.comingSoon}</small></Show></span><Show when={index() === 0}><Icon name="check"/></Show></button>}</For><button class="settings-menu-item" onClick={() => router.navigate('/settings/devices')}><Icon name="settings"/><span>{s.settings}</span></button></div></Show></div></footer></aside><div ref={resizer} class="pane-resizer" role="separator" aria-label={s.resizeSidebar} aria-orientation="vertical" tabindex="0" onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault();
        const width = sidebar.getBoundingClientRect().width + (event.key === 'ArrowRight' ? 10 : -10);
        document.documentElement.style.setProperty('--sidebar-width', Math.max(280, Math.min(420, width)) + 'px');
    } }}/><main ref={transcript} inert={phoneDetails()} class="transcript" data-scroll="transcript"><header class="toolbar glass"><button class="icon-button back-button" aria-label={s.back} onClick={() => router.back()}><Icon name="chevron"/></button><span class="toolbar-title"><Show when={threadRoute()} fallback={chat()==='new'?s.compose:''}><span class="chat-avatar" aria-hidden="true">{threadTitle().split(/\s+/u).slice(0,2).map(value=>Array.from(value)[0]??'').join('')}</span><span>{threadTitle()}</span></Show></span><button class="icon-button" aria-label={s.details} aria-expanded={details()||router.route().panel==='details'} onClick={() => openInspector(!details())}><Icon name="info"/></button></header><div class="transcript-body"><Show when={threadRoute()?.chatID} keyed fallback={<div class="empty quiet-empty"><p>{s.noSelection}</p></div>}>{id=><Suspense><Transcript chatID={id} messageID={threadRoute()?.messageID} title={setThreadTitle}/></Suspense>}</Show></div></main><Show when={inspectorVisible() && !compact()}><aside ref={inspector} class="inspector" aria-label={s.details}><header class="toolbar glass"><span class="toolbar-title">{s.details}</span><button class="icon-button" aria-label={s.closeDetails} onClick={() => openInspector(false)}><Icon name="close"/></button></header><DetailContent /></aside></Show><Show when={!phone()&&compact()&&(details()||router.route().panel==='details')}><dialog ref={element => queueMicrotask(() => element.showModal())} class="details-sheet glass" aria-label={s.details} onClose={() => openInspector(false)}><button autofocus class="icon-button sheet-close" aria-label={s.closeDetails} onClick={() => setDetails(false)}><Icon name="close"/></button><DetailContent /></dialog></Show><Show when={phoneDetails()}><PhoneDetails/></Show></div>;
}
