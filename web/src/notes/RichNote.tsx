import { createMemo, For, Match, Show, Switch } from 'solid-js';
import { object, text } from './store';
const array = (value: unknown) => Array.isArray(value) ? value : [];
const highlights = new Set(['purple', 'pink', 'orange', 'mint', 'blue']);
function safeLink(value: unknown) {
    const href = text(value);
    try { return ['https:', 'http:', 'mailto:', 'tel:'].includes(new URL(href).protocol) ? href : undefined; } catch { return; }
}
function Attachment(props: { id: string; attachments: unknown[] }) {
    const metadata = () => props.attachments.map(object).find(value => text(value.attachment_id ?? value.id) === props.id);
    return <span class="notes-attachment">{text(metadata()?.filename ?? metadata()?.name) || 'Attachment unavailable'}</span>;
}
function Inline(props: { value: unknown; attachments: unknown[] }) {
    const run = () => object(props.value), highlight = () => highlights.has(text(run().highlight)) ? text(run().highlight) : '';
    const styles = () => {
        const value = run(), size = Number(value.font_size), font = text(value.font), color = text(value.color);
        return {
            'font-weight': value.bold ? '700' : undefined,
            'font-style': value.italic ? 'italic' : undefined,
            'text-decoration': [value.underline ? 'underline' : '', value.strikethrough ? 'line-through' : ''].filter(Boolean).join(' ') || undefined,
            'font-family': /^[\p{L}\p{N} ._-]{1,128}$/u.test(font) ? `"${font}", system-ui` : undefined,
            'font-size': Number.isFinite(size) && size > 0 && size <= 512 ? `${size}pt` : undefined,
            color: /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/iu.test(color) ? color : undefined,
            'vertical-align': Number(value.superscript) > 0 ? 'super' : Number(value.superscript) < 0 ? 'sub' : undefined
        };
    };
    const content = () => <span style={styles()}>{text(run().text).replace(/\ufffc/gu, '')}<Show when={text(run().attachment_id)}><Attachment id={text(run().attachment_id)} attachments={props.attachments}/></Show></span>;
    const marked = () => <Show when={highlight()} fallback={content()}><mark class={'notes-highlight-' + highlight()}>{content()}</mark></Show>;
    return <Show when={safeLink(run().link)} fallback={marked()}><a href={safeLink(run().link)} rel="noreferrer" target="_blank">{marked()}</a></Show>;
}
function Content(props: { block: Record<string, unknown>; attachments: unknown[] }) {
    return <Show when={array(props.block.runs).length} fallback={text(props.block.text)}><For each={array(props.block.runs)}>{run => <Inline value={run} attachments={props.attachments}/>}</For></Show>;
}
function Blocks(props: { values: unknown[]; attachments: unknown[] }) {
    const groups = createMemo(() => {
        const result: { kind: string; values: Record<string, unknown>[] }[] = [];
        for (const value of props.values) {
            const block = object(value), kind = block.type === 'list_item' ? 'list:' + text(block.list_style) : block.type === 'checklist_item' ? 'checklist' : '';
            const previous = result.at(-1);
            if (kind && previous?.kind === kind) previous.values.push(block);
            else result.push({ kind, values: [block] });
        }
        return result;
    });
    const indent = (block: Record<string, unknown>) => Number.isInteger(block.level) && Number(block.level) > 0 && Number(block.level) <= 128 ? Number(block.level) + 'em' : undefined;
    const listItems = (values: Record<string, unknown>[]) => <For each={values}>{block => <li style={{ 'margin-inline-start': indent(block) }}><Content block={block} attachments={props.attachments}/><Blocks values={array(block.children)} attachments={props.attachments}/></li>}</For>;
    return <For each={groups()}>{group => <Switch fallback={<For each={group.values}>{block => <Block value={block} attachments={props.attachments}/>}</For>}>
        <Match when={group.kind === 'list:numbered'}><ol>{listItems(group.values)}</ol></Match>
        <Match when={group.kind.startsWith('list:')}><ul classList={{ 'notes-dashed-list': group.kind === 'list:dash' }}>{listItems(group.values)}</ul></Match>
        <Match when={group.kind === 'checklist'}><ul class="notes-checklist"><For each={group.values}>{block => <li style={{ 'margin-inline-start': indent(block) }}><Show when={typeof block.checked === 'boolean'} fallback={<span aria-label="Checklist status unavailable">—</span>}><input type="checkbox" disabled checked={block.checked === true} aria-label={text(block.text)}/></Show><div><Content block={block} attachments={props.attachments}/><Blocks values={array(block.children)} attachments={props.attachments}/></div></li>}</For></ul></Match>
    </Switch>}</For>;
}
function Block(props: { value: Record<string, unknown>; attachments: unknown[] }) {
    const block = () => props.value, kind = () => text(block().type);
    const content = () => <Content block={block()} attachments={props.attachments}/>;
    const children = () => <Blocks values={array(block().children)} attachments={props.attachments}/>;
    const heading = () => <Show when={block().level === 2} fallback={<h2>{content()}</h2>}><h3>{content()}</h3></Show>;
    const alignment = () => ['left', 'center', 'right', 'justify'].includes(text(block().alignment)) ? text(block().alignment) as 'left' | 'center' | 'right' | 'justify' : undefined;
    return <div class="notes-block" classList={{ 'notes-blockquote': block().blockquote === true && kind() !== 'quote' }} style={{ 'text-align': alignment() }}><Switch fallback={<p class="notes-unknown">Unsupported note block</p>}>
        <Match when={kind() === 'title'}><h1>{content()}</h1>{children()}</Match>
        <Match when={kind() === 'heading'}><Show when={block().collapsible === true} fallback={<>{heading()}{children()}</>}><details open><summary>{heading()}</summary>{children()}</details></Show></Match>
        <Match when={kind() === 'paragraph'}><p>{content()}</p>{children()}</Match>
        <Match when={kind() === 'monostyled'}><pre>{content()}</pre>{children()}</Match>
        <Match when={kind() === 'quote'}><blockquote>{content()}{children()}</blockquote></Match>
        <Match when={kind() === 'table'}><div class="notes-table"><table><tbody><For each={array(block().rows)}>{row => <tr><For each={array(row)}>{cell => <td><Blocks values={array(object(cell).blocks)} attachments={props.attachments}/></td>}</For></tr>}</For></tbody></table></div>{children()}</Match>
        <Match when={kind() === 'attachment'}><p><Attachment id={text(block().attachment_id)} attachments={props.attachments}/></p>{children()}</Match>
    </Switch></div>;
}
export function RichNote(props: { data: Record<string, unknown> }) {
    const plain = () => { const value = text(props.data.plain_text), lines = value.split('\n'); return lines[0] === props.data.title ? lines.slice(1).join('\n') : value; };
    return <div class="notes-rich"><Show when={array(props.data.blocks).length} fallback={<p class="notes-plain">{plain() || (props.data.content_status === 'complete' ? 'Empty note' : 'Content unavailable on this device.')}</p>}><Blocks values={array(props.data.blocks)} attachments={array(props.data.attachments)}/></Show></div>;
}
