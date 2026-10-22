import { batch, createEffect, createSignal, onCleanup } from 'solid-js';
import type { LiveClient } from '../../data/live';
import { withInput } from '../../data/input';
import type { CommandPart, CommandState, Cursor, ThreadItem } from '../../data/types';
import { createCapabilities } from './capabilities';
export type MessageFile = { upload_id: string; name: string };
export type SendMessage = { chat_id: string; text?: string; files?: MessageFile[]; handles?: string[]; service?: string; resend_of?: string };
export type MessageSendState = 'pending' | 'sent' | 'failed' | 'indeterminate';
export type DeliveryNotice = { count: number; command_ids: readonly string[]; text: string };
type Echo = { commandID: string; index: number; kind: 'text' | 'attachment'; legacy: boolean; item: ThreadItem };
function messageSendState(command: CommandState): MessageSendState {
    if (command.status === 'indeterminate') return 'indeterminate';
    if (['failed', 'rejected', 'delivery_failed', 'expired'].includes(command.status)) return 'failed';
    return ['observed', 'delivered'].includes(command.status) ? 'sent' : 'pending';
}
export function messageCommandLabel(command: CommandState) {
    return { pending: 'Sending…', sent: 'Sent', failed: 'Not Delivered', indeterminate: 'Not sure if delivered' }[messageSendState(command)];
}
export function createMessageCommands(live: LiveClient) {
    const capabilities = createCapabilities(live);
    const replies = new Map<string,{target:ThreadItem;partIndex:number}>();
    const echoes = new Map<string, Echo>(), owned = new Map<string, SendMessage>(), suppressed = new Set<string>(), resolved = new Set<string>();
    const identities = new Map<string, string>(), displayed = new Map<string, string>(), held = new Map<string, string>();
    const snapshots = new Map<string, { ids: Set<string>; cursor?: Cursor }>();
    const released = new Map<string, Set<string>>(), missingIdentities = new Set<string>();
    const timers = new Map<string, ReturnType<typeof setTimeout>>(), retries = new Map<string,string>();
    const [revision, setRevision] = createSignal(0), [timeoutCount, setTimeoutCount] = createSignal(0), [missingIdentityCount, setMissingIdentityCount] = createSignal(0);
    let disposed = false;
    const changed = () => setRevision(value => value + 1);
    const chatOf = (echo: Echo) => String(echo.item.data.chat_id ?? '');
    const ambiguous = (key: string) => (released.get(key)?.size ?? 0) > 0;
    const partOf = (echo: Echo, command: CommandState | undefined): CommandPart | undefined => command?.parts?.[echo.index];
    const stateOf = (echo: Echo, command: CommandState | undefined): MessageSendState => {
        if (!command) return 'pending';
        const part = partOf(echo, command);
        if (part) {
            if (['observed','delivered'].includes(part.disposition)) return 'sent';
            if (['delivery_failed','failed_no_effect'].includes(part.disposition)) return 'failed';
            if (['observed','delivered'].includes(command.status)) return 'sent';
            return ['accepted','queued','preparing','prepared','sent'].includes(command.status) && !command.disposition ? 'pending' : 'indeterminate';
        }
        if (!echo.legacy && (command.status === 'delivery_failed' || command.disposition === 'delivery_failed')) return 'indeterminate';
        return messageSendState(command);
    };
    const recordOf = (echo: Echo, command: CommandState | undefined) => command?.parts ? partOf(echo, command)?.record_id : echo.legacy && command?.observed_record_ids?.length === 1 ? command.observed_record_ids[0] : undefined;
    const ghostFailure = (echo: Echo, command: CommandState | undefined) => !recordOf(echo, command) && (partOf(echo, command)?.disposition === 'delivery_failed' || echo.legacy && !command?.parts && (command?.status === 'delivery_failed' || command?.disposition === 'delivery_failed'));
    const hasEcho = (chatID: string, commands: Map<string, CommandState>) => [...echoes].some(([key, echo]) => chatOf(echo) === chatID && !suppressed.has(key) && !resolved.has(key) && stateOf(echo, commands.get(echo.commandID)) !== 'failed');
    const release = (chatID: string) => {
        clearTimeout(timers.get(chatID)); timers.delete(chatID);
        for (const [id, chat] of held) if (chat === chatID) { held.delete(id); displayed.set(id, chatID); }
    };
    const compatible = (command: CommandState, cursor: Cursor | undefined) => Boolean(command.commit_seq && command.cursor && cursor && cursor.epoch === command.cursor.epoch && cursor.subscription_revision === command.cursor.subscription_revision && cursor.sequence >= command.commit_seq);
    const rememberCandidates = (chatID: string, commands: Map<string, CommandState>, all: boolean) => {
        const candidates = [...held].filter(([, chat]) => chat === chatID).map(([id]) => id);
        if (!candidates.length) return;
        for (const [key, echo] of echoes) {
            const command = commands.get(echo.commandID);
            if (chatOf(echo) !== chatID || resolved.has(key) || ambiguous(key) || !(all && stateOf(echo, command) !== 'failed' || ghostFailure(echo, command))) continue;
            released.set(key, new Set([...(released.get(key) ?? []), ...candidates])); suppressed.add(key);
        }
    };
    const refresh = () => {
        if (disposed) return;
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [key, echo] of echoes) {
            const command = commands.get(echo.commandID);
            if (!command) continue;
            const recordID = recordOf(echo, command);
            if (!chatOf(echo) && recordID) {
                const actual = [...snapshots].find(([, value]) => value.ids.has(recordID) && compatible(command, value.cursor));
                if (actual) echo.item = {...echo.item, data:{...echo.item.data,chat_id:actual[0]}};
            }
            const capture = snapshots.get(chatOf(echo)), state = stateOf(echo, command);
            if (state === 'sent' && !recordID && !missingIdentities.has(key)) { missingIdentities.add(key); setMissingIdentityCount(value => value + 1); }
            if (recordID && compatible(command, capture?.cursor)) {
                if (resolved.has(key) && !capture!.ids.has(recordID)) {
                    if (identities.get(recordID) === key) identities.delete(recordID);
                    displayed.delete(recordID); echoes.delete(key); retries.delete(key); resolved.delete(key); suppressed.delete(key); released.delete(key); missingIdentities.delete(key);
                    if (![...echoes.values()].some(value => value.commandID === echo.commandID)) {owned.delete(echo.commandID);replies.delete(echo.commandID);}
                    continue;
                }
                if (capture!.ids.has(recordID)) {
                    identities.set(recordID, key); held.delete(recordID); displayed.set(recordID, chatOf(echo));
                    for (const candidates of released.values()) candidates.delete(recordID);
                    resolved.add(key);
                }
            }
            const releasedRecord = recordID && released.get(key)?.has(recordID) && displayed.has(recordID);
            if (state === 'failed' && !ghostFailure(echo, command) && !releasedRecord || (state === 'indeterminate' || ghostFailure(echo, command)) && !ambiguous(key)) suppressed.delete(key);
        }
        for (const chatID of snapshots.keys()) {
            if (!hasEcho(chatID, commands)) { rememberCandidates(chatID, commands, false); release(chatID); continue; }
            if (![...held.values()].includes(chatID)) { clearTimeout(timers.get(chatID)); timers.delete(chatID); continue; }
            if (timers.has(chatID)) continue;
            timers.set(chatID, setTimeout(() => batch(() => {
                if (disposed) return;
                rememberCandidates(chatID, new Map(live.pending().map(command => [command.id, command])), true);
                release(chatID); setTimeoutCount(value => value + 1); changed();
            }), 500));
        }
        changed();
    };
    createEffect(refresh);
    const observe = (items: readonly ThreadItem[], cursor: Cursor | undefined, chatID?: string) => {
        const chats = new Map<string, ThreadItem[]>();
        if (chatID) chats.set(chatID, []);
        for (const item of items) {
            const id = String(item.data.chat_id ?? '');
            if (!chats.has(id)) chats.set(id, []);
            chats.get(id)!.push(item);
        }
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [id, captured] of chats) {
            const current = new Set(captured.map(item => item.id));
            if (cursor) {
                for (const [recordID, chat] of displayed) if (chat === id && !current.has(recordID)) displayed.delete(recordID);
                for (const [recordID, chat] of held) if (chat === id && !current.has(recordID)) held.delete(recordID);
            }
            snapshots.set(id, {ids:current,cursor});
            const holding = hasEcho(id, commands);
            for (const item of captured) {
                if (!holding || displayed.has(item.id) || identities.has(item.id) || !item.data.is_from_me) { displayed.set(item.id, id); held.delete(item.id); }
                else held.set(item.id, id);
            }
        }
        refresh();
    };
    const projectCaptured = (items: readonly ThreadItem[], _cursor: Cursor | undefined): readonly ThreadItem[] => {
        revision();
        if (!held.size && !identities.size) return items;
        const commands = new Map(live.pending().map(command => [command.id, command]));
        return items.filter(item => !held.has(item.id)).map(item => {
            const echo = echoes.get(identities.get(item.id) ?? '');
            return echo ? {...item,data:{...item.data,command_id:echo.commandID,...(!echo.legacy ? {command_part_index:echo.index} : {}),command_sent_at:echo.item.data.sent_at,send_state:stateOf(echo,commands.get(echo.commandID))}} : item;
        });
    };
    const pending = (): readonly ThreadItem[] => {
        revision();
        const commands = new Map(live.pending().map(command => [command.id, command]));
        return [...echoes].filter(([key]) => !suppressed.has(key) && !resolved.has(key)).map(([, echo]) => ({...echo.item,data:{...echo.item.data,send_state:stateOf(echo,commands.get(echo.commandID))}}));
    };
    const addEchoes=(id:string,submitted:SendMessage,replyData:Record<string,unknown>={})=>{
        const sentAt=new Date().toISOString(),legacy=!submitted.files?.length;
        const content: {kind:'text'|'attachment';text:string}[] = [...(submitted.text ? [{kind:'text' as const,text:submitted.text}] : []), ...(submitted.files ?? []).map(() => ({kind:'attachment' as const,text:''}))];
        owned.set(id, submitted);
        content.forEach((part, index) => {
            const key = legacy ? id : id + ':part:' + index;
            echoes.set(key, {commandID:id,index,kind:part.kind,legacy,item:{id:'command:'+key,collection:'messages.message',source:'messages',version:0,
                data:{command_id:id,...(!legacy ? {command_part_index:index} : {}),command_sent_at:sentAt,chat_id:submitted.chat_id,text:part.text,parts:part.kind === 'text' ? [{index:0,kind:'text',text:part.text}] : [],is_from_me:true,sent_at:sentAt,send_state:'pending',...replyData},attachments:[],reactions:[],reply_count:0}});
        });
        refresh();
    };
    const send = (event: Event, args: SendMessage) => withInput(event, () => {
        if (!capabilities.allowed('messages.send') || !args.text && !args.files?.length || (args.files?.length ?? 0) > 10) throw new Error('Sending is unavailable');
        const submitted = {...args,...(args.files ? {files:args.files.map(file => ({...file}))} : {}),...(args.handles ? {handles:args.handles.slice()} : {})};
        const id=live.command('messages.send',submitted);
        addEchoes(id,submitted);
        return id;
    });
    const reply=(event:Event,target:ThreadItem,partIndex:number,text:string)=>withInput(event,()=>{
        const parts=Array.isArray(target.data.parts)?target.data.parts:[];
        if(!capabilities.allowed('messages.reply')||target.collection!=='messages.message'||target.version<1||target.id.startsWith('command:')||target.data.retracted_at||target.data.deleted_at||typeof target.data.chat_id!=='string'||!target.data.chat_id||!Number.isSafeInteger(partIndex)||partIndex<0||!parts.some(part=>part?.index===partIndex)||Array.isArray(target.data.retracted_parts)&&target.data.retracted_parts.includes(partIndex)||!text.trim()||new TextEncoder().encode(text).length>65536)throw new Error('Reply unavailable');
        const id=live.command('messages.reply',{id:target.id,part_index:partIndex,text});
        replies.set(id,{target:{...target,data:{...target.data}},partIndex});
        addEchoes(id,{chat_id:String(target.data.chat_id??''),text},{reply_to_id:target.id,thread_originator_id:typeof target.data.thread_originator_id==='string'&&target.data.thread_originator_id?target.data.thread_originator_id:target.id});
        return id;
    });
    const echoKey = (id:string,index?:number) => index === undefined ? id : Number.isSafeInteger(index) && index >= 0 ? id+':part:'+index : '';
    const canRetry = (id:string,index?:number,file?:MessageFile) => {
        revision();
        const key=echoKey(id,index), echo=echoes.get(key), command=live.pending().find(value => value.id===id);
        return Boolean(echo && command && !retries.has(key) && stateOf(echo,command)==='failed' && !ghostFailure(echo,command) && (!recordOf(echo,command) || chatOf(echo)) && (echo.kind==='text' || file?.upload_id && file.name) && live.status()==='connected' && (!replies.has(id)||!recordOf(echo,command)) && capabilities.allowed(replies.has(id)?'messages.reply':'messages.send'));
    };
    const retry = (event:Event,id:string,index?:number,file?:MessageFile) => withInput(event,() => batch(() => {
        const key=echoKey(id,index), existing=retries.get(key);
        if(existing)return existing;
        if(!canRetry(id,index,file))throw new Error('Message unavailable');
        const echo=echoes.get(key)!, original=owned.get(id)!, command=live.pending().find(value => value.id===id), recordID=recordOf(echo,command), chatID=chatOf(echo);
        const context=replies.get(id);
        const next=context?reply(event,context.target,context.partIndex,String(echo.item.data.text??'')):send(event,{chat_id:chatID,...(!chatID ? {handles:original.handles?.slice(),service:original.service} : {}),...(echo.kind==='text' ? {text:String(echo.item.data.text ?? '')} : {files:[{upload_id:file!.upload_id,name:file!.name}]}),...(recordID ? {resend_of:recordID} : {})});
        retries.set(key,next);
        if(!recordID){echoes.delete(key);retries.delete(key);suppressed.delete(key);released.delete(key);missingIdentities.delete(key);if(![...echoes.values()].some(value => value.commandID===id)){owned.delete(id);replies.delete(id);}}
        refresh();
        return next;
    }));
    const statuses = (chatID?:string) => { revision(); return live.pending().filter(command => owned.has(command.id)&&(!chatID||owned.get(command.id)?.chat_id===chatID)); };
    const notice = (chatID: string): DeliveryNotice | undefined => {
        revision();
        const commands = new Map(statuses().map(command => [command.id, command]));
        const uncertain: Echo[] = [], failures: Echo[] = [];
        for (const [key, echo] of echoes) {
            const command = commands.get(echo.commandID);
            if (chatOf(echo) !== chatID || resolved.has(key) || !suppressed.has(key) || !ambiguous(key)) continue;
            if (ghostFailure(echo, command)) failures.push(echo);
            else if (stateOf(echo, command) === 'indeterminate') uncertain.push(echo);
        }
        const failedChecks = [...commands.values()].filter(command => command.check_error && [...echoes.values()].some(echo => echo.commandID === command.id && chatOf(echo) === chatID));
        const ids = [...new Set([...uncertain,...failures].map(echo => echo.commandID).concat(failedChecks.map(command => command.id)))];
        const text = [failures.length ? failures.length === 1 ? "1 message wasn't delivered" : failures.length+" messages weren't delivered" : '',
            uncertain.length ? uncertain.length === 1 ? '1 message may not have been delivered' : uncertain.length+' messages may not have been delivered' : '',
            failedChecks.length ? failedChecks.length === 1 ? "Couldn't check this message. Try again." : "Couldn't check these messages. Try again." : ''].filter(Boolean).join(' · ');
        return ids.length ? {count:uncertain.length+failures.length || failedChecks.length,command_ids:ids,text} : undefined;
    };
    const copy = (event: Event, id: string, partIndex?: number) => withInput(event, () => {
        if (partIndex !== undefined && (!Number.isSafeInteger(partIndex) || partIndex < 0)) throw new Error('Message unavailable');
        const echo = partIndex === undefined ? echoes.get(id) : echoes.get(id+':part:'+partIndex);
        if (!echo || echo.kind === 'attachment') throw new Error('Message unavailable');
        return navigator.clipboard.writeText(String(echo.item.data.text ?? ''));
    });
    const dispose = () => { disposed = true; for (const timer of timers.values()) clearTimeout(timer); timers.clear(); snapshots.clear(); echoes.clear(); owned.clear(); identities.clear(); held.clear(); displayed.clear(); released.clear(); suppressed.clear(); resolved.clear(); missingIdentities.clear(); retries.clear(); replies.clear(); capabilities.dispose(); };
    onCleanup(dispose);
    return {send,reply,retry,canRetry,pending,observe,projectCaptured,statuses,notice,timeoutCount,missingIdentityCount,capabilities,check:(id:string)=>live.checkCommand(id),copy,dispose};
}
