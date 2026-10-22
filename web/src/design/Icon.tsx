export type IconName = 'laptop' | 'appearance' | 'list' | 'search' | 'ellipsis' | 'messages' | 'reminders' | 'calendar' | 'contacts' | 'notes' | 'settings' | 'chevron' | 'grid' | 'key' | 'devices' | 'spark' | 'shield' | 'activity' | 'sun' | 'check' | 'close' | 'compose' | 'info' | 'share' | 'download' | 'phone' | 'video' | 'mail';
export function Icon(props: {
    name: IconName;
    class?: string;
}) {
    return <svg viewBox="0 0 24 24" class={'icon ' + (props.class ?? '')} aria-hidden="true"><use width="24" height="24" href={'/icons.svg#' + props.name}/></svg>;
}
