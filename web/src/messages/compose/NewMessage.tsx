import { createEffect, createMemo, createSignal, For, lazy, onCleanup, onMount, Show, Suspense, untrack } from 'solid-js';
import { auth } from '../../auth/session';
import { router } from '../../app/router';
import type { createMessageCommands } from '../actions/commands';
import { messageCommandLabel } from '../actions/commands';
import { Composer } from './Composer';
import {UploadDock} from './UploadDock';
import type {Uploads} from './uploads';
import type {PendingAttachment} from '../media/pending';
import {PendingMedia} from '../media/PendingMedia';
import {takeForward,discardDraft} from './drafts';
import { createRecipientHistory } from './history';
import { lookupRecipients, rawRecipient, recipientKey, recipientOptions, type Recipient } from './recipients';
import './new-message.css';
const Transcript = lazy(() => import('../transcript/Transcript'));
export default function NewMessage(props: {commands:ReturnType<typeof createMessageCommands>;uploads:Uploads}) {
    const live = auth.live()!, session = auth.session()!, commands = props.commands;
    const incoming = rawRecipient(new URLSearchParams(location.search).get('to') ?? '');
    const forward=takeForward(session),draftID=forward?.draftID??"new-message";
    onCleanup(()=>{if(forward){discardDraft(session,forward.draftID);props.uploads.cancelImport(forward.draftID);for(const entry of props.uploads.files(forward.draftID))props.uploads.remove(forward.draftID,entry.key);}});
    const recover = untrack(() => incoming || forward ? undefined : commands.statuses().findLast(command => {
        const args = command.args as {chat_id?:string;handles?:string[]};
        return !args.chat_id && args.handles?.length === 1 && !['observed','delivered'].includes(command.status);
    }));
    const recovered = recover?.args as {handles?:string[]} | undefined;
    const [recipients,setRecipients] = createSignal<Recipient[]>(recovered?.handles?.flatMap(value => rawRecipient(value) ?? []) ?? (incoming ? [incoming] : []));
    const [query,setQuery] = createSignal(''), [options,setOptions] = createSignal<Recipient[]>([]), [active,setActive] = createSignal(-1), [open,setOpen] = createSignal(false), [lookupHint,setLookupHint] = createSignal('');
    const [commandID,setCommandID] = createSignal(recover?.id ?? ''), [startedChat,setStartedChat] = createSignal('');
    let field!: HTMLInputElement;
    const latest = () => commands.statuses().find(command => command.id === commandID());
    const history = createRecipientHistory(live,() => recipients().map(recipient => recipient.value));
    const knownChat = createMemo(() => history.rows()[0]);
    const service = () => String(knownChat()?.chat.data.service ?? '');
    const existing = () => commandID() && !startedChat() && !['observed','delivered'].includes(latest()?.status ?? '') ? undefined : knownChat();
    const capable = () => commands.capabilities.allowed('messages.send');
    const blocked = () => recipients().length !== 1 || Boolean(commandID());
    const local = () => existing() ? [] : commands.pending().filter(item => item.data.command_id === commandID());
    onMount(() => field.focus());
    createEffect(() => {
        const q = query().trim(); live.status();
        setOptions([]); setActive(-1); setLookupHint('');
        if (!q || Array.from(q).length > 64) { setOpen(false); return; }
        const abort = new AbortController();
        const timer = setTimeout(() => {
            void lookupRecipients(q,abort.signal).then(result => {
                if (abort.signal.aborted || untrack(query).trim() !== q) return;
                setOptions(recipientOptions(result.results,q));
                setOpen(true);
                if (result.capability === 'unavailable') setLookupHint('Enter a phone number or email address');
            }).catch(() => { if (!abort.signal.aborted) { setOpen(false); setLookupHint("Can't look up contacts right now. Enter a phone number or email address."); } });
        },80);
        onCleanup(() => { clearTimeout(timer); abort.abort(); });
    });
    createEffect(() => {
        const command = latest(), id = ['observed','delivered'].includes(command?.status ?? '') ? command?.observed_record_ids?.[0] : undefined;
        if (!id) return;
        const record = live.subscribe('record',{collection:'messages.message',id},{cache:false,resetOnReconnect:true});
        createEffect(() => {
            const value = record.data().records?.find(row => row.id === id), cursor = record.cursor();
            if (!value || !cursor || !command?.cursor || cursor.epoch !== command.cursor.epoch || cursor.subscription_revision !== command.cursor.subscription_revision || cursor.sequence < (command.commit_seq ?? Infinity)) return;
            if (typeof value.data.chat_id === 'string') router.navigate('/messages/'+encodeURIComponent(value.data.chat_id),true);
        });
        onCleanup(record.dispose);
    });
    const choose = (recipient: Recipient) => {
        if (!recipients().some(value => recipientKey(value.value) === recipientKey(recipient.value))) setRecipients(values => [...values,recipient]);
        setQuery(''); setOptions([]); setOpen(false); setLookupHint(''); field.focus();
    };
    const commit = () => {
        const option = options()[active() >= 0 ? active() : 0], raw = rawRecipient(query());
        if (open() && active() >= 0 && option) { choose(option); return true; }
        if (raw) { choose(options().find(value => recipientKey(value.value) === recipientKey(raw.value)) ?? raw); return true; }
        if (open() && option) { choose(option); return true; }
        return false;
    };
    const send = (event: Event, content: {text:string}) => {
        if (!capable() || blocked()) throw new Error('Sending unavailable');
        const files=props.uploads.files(draftID).length?props.uploads.prepared(draftID):[];
        const chat = knownChat(), args = {chat_id:chat?.chat.id ?? '',text:content.text,...(files.length?{files:files.map(file=>file.upload)}:{}),...(chat ? {} : {handles:[recipients()[0].value],service:service() || 'iMessage'})};
        const id = commands.send(event,args,files);props.uploads.sent(draftID,files);
        setStartedChat(chat?.chat.id ?? ''); setCommandID(id);
        return id;
    };
    return <section onDragOver={event=>{if(event.dataTransfer?.types.includes("Files"))event.preventDefault();}} onDrop={event=>{if(event.defaultPrevented||!event.dataTransfer?.files.length)return;event.preventDefault();props.uploads.add(draftID,Array.from(event.dataTransfer.files));}} class="new-message-pane" aria-label="New message">
        <div class="recipient-header">
            <span class="recipient-to">To:</span>
            <div class="recipient-entry">
                <ul class="recipient-tokens" aria-label="Recipients"><For each={recipients()}>{recipient => <li data-recipient-value={recipient.value}><span title={recipient.value}>{recipient.display_name}</span><button type="button" disabled={Boolean(commandID())} aria-label={'Remove '+recipient.display_name} onClick={() => setRecipients(values => values.filter(value => value !== recipient))}>×</button></li>}</For></ul>
                <input ref={field} disabled={Boolean(commandID())} role="combobox" aria-label="To" aria-autocomplete="list" aria-expanded={open() && options().length > 0} aria-controls="recipient-options" aria-activedescendant={open() && active() >= 0 ? 'recipient-option-'+active() : undefined} value={query()} autocomplete="off" inputmode="email" onInput={event => setQuery(event.currentTarget.value)} onFocus={() => { if (options().length) setOpen(true); }} onKeyDown={event => {
                    if (event.isComposing || event.keyCode === 229 || event.repeat) return;
                    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { if (!options().length) return; event.preventDefault(); setOpen(true); setActive(value => value < 0 ? event.key === 'ArrowDown' ? 0 : options().length-1 : (value+(event.key === 'ArrowDown' ? 1 : options().length-1)+options().length)%options().length); }
                    else if (event.key === 'Escape') { setOpen(false); setActive(-1); }
                    else if (event.key === 'Enter' || event.key === ',' || event.key === ';' || event.key === 'Tab') { if (commit() && event.key !== 'Tab') event.preventDefault(); }
                    else if (event.key === 'Backspace' && !query()) setRecipients(values => values.slice(0,-1));
                }}/>
            </div>
            <Show when={recipients().length === 1}><span class="recipient-service" classList={{imessage:service() === 'iMessage',text:['SMS','RCS'].includes(service())}} role="status" aria-label="Message service">{history.ready() || history.failed() ? service() || 'Unknown' : 'Checking history…'}</span></Show>
            <Show when={open() && options().length}><ul id="recipient-options" class="recipient-options glass" role="listbox" aria-label="Contacts"><For each={options()}>{(option,index) => <li id={'recipient-option-'+index()} role="option" aria-selected={active() === index()} onPointerDown={event => event.preventDefault()} onClick={() => choose(option)}><strong>{option.display_name}</strong><span>{option.label ? option.label+' · ' : ''}{option.value}</span></li>}</For></ul></Show>
        </div>
        <Show when={lookupHint()}><p class="recipient-hint" role="status">{lookupHint()}</p></Show>
        <Show when={history.failed()}><p class="recipient-hint">Couldn't load earlier messages</p></Show>
        <div class="new-message-history"><Show when={existing()?.chat.id} keyed fallback={<div class="new-message-empty"/>}>{chatID => <section class="new-existing" aria-label="Existing conversation" data-chat-id={chatID}><Suspense><Transcript chatID={chatID} title={() => {}}/></Suspense></section>}</Show><Show when={local().length}><div class="new-pending" role="log" aria-label="Pending messages"><For each={local()}>{item => <div class="new-pending-row"><For each={Array.isArray(item.data.local_attachments)?item.data.local_attachments as PendingAttachment[]:[]}>{attachment=><div style={{position:"relative",width:"260px",height:"120px"}}><PendingMedia attachment={attachment} width={260} height={120}/></div>}</For><p>{String(item.data.text ?? '')}</p><small>{latest() ? messageCommandLabel(latest()!) : 'Sending…'}</small></div>}</For></div></Show></div>
        <UploadDock uploads={props.uploads} draftID={draftID}><Composer session={session} draftID={draftID} hasAttachments={props.uploads.files(draftID).length>0} service={service()} available={capable()} disabled={blocked()||props.uploads.busy(draftID)||props.uploads.files(draftID).length>0&&!props.uploads.ready(draftID)} disabledReason={recipients().length > 1 ? "New group messages aren't available yet" : undefined} command={latest()} canCheck={live.status()==='connected'} onSend={send} onCheck={() => commands.check(commandID())} onCopy={commands.canCopy(commandID())?event => { void commands.copy(event,commandID()); }:undefined}/></UploadDock>
    </section>;
}
