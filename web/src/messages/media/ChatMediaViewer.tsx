import {lazy,Suspense} from 'solid-js';
import {Portal} from 'solid-js/web';
import type {MediaViewerProps} from './MediaViewer';
export type ChatMediaViewerProps=Omit<MediaViewerProps,'attachments'|'loading'|'error'|'onRetry'|'shareUrl'|'onSelection'|'senderName'|'timestamp'|'subtitle'|'originFor'> & {sourceRoot?:ParentNode;chatId:string;pageSize?:number;locale?:string;now?:Date};
let pending:Promise<typeof import('./ViewerSession')>|undefined;
export function preloadMediaViewer(){return pending??=import('./ViewerSession').catch(error=>{pending=undefined;throw error;});}
const Viewer=lazy(()=>preloadMediaViewer().then(module=>({default:module.ViewerSession})));
export function ChatMediaViewer(props:ChatMediaViewerProps){return <Portal><Suspense fallback={<span class="media-viewer-pending" role="status">Opening media…</span>}><Viewer {...props}/></Suspense></Portal>;}
