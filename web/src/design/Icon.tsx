export type IconName = 'messages' | 'reminders' | 'calendar' | 'contacts' | 'notes' | 'settings' | 'chevron' | 'grid' | 'key' | 'devices' | 'spark' | 'shield' | 'activity' | 'sun' | 'check' | 'close' | 'compose' | 'info';
export function Icon(props: { name: IconName; class?: string }) {
  return <svg class={'icon ' + (props.class ?? '')} aria-hidden="true"><use href={'/icons.svg#' + props.name}/></svg>;
}
