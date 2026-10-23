import {createSignal,onCleanup} from 'solid-js';
import {createMutable} from 'solid-js/store';
import type {BridgeRecord} from '../../data/types';
import {attachmentOriginal,attachmentFilename} from '../media/files';
import type {LocalMessageFile} from '../actions/commands';
import type {PendingAttachment} from '../media/pending';
type UploadReply={upload_id:string;offset:number;size:number;state:'uploading'|'ready';expires_at:string};
export type StagedFile={key:string;file:File;attachment:PendingAttachment;phase:'uploading'|'finishing'|'paused'|'ready';bytes:number;remote?:UploadReply;problem:string;abort:AbortController;generation:number;metadata:boolean};
const maximum=100*1024*1024,chunkSize=1024*1024;
class UploadError extends Error {constructor(readonly status:number,readonly retryAfter:number=1000){super('Upload unavailable');}}
function request(method:string,path:string,body:Blob|undefined,headers:Record<string,string>,signal:AbortSignal,progress?:(bytes:number,finished:boolean)=>void):Promise<UploadReply>{
    return new Promise((resolve,reject)=>{
        const xhr=new XMLHttpRequest();xhr.open(method,path);xhr.withCredentials=true;xhr.timeout=0;xhr.responseType='json';
        for(const [name,value] of Object.entries(headers))xhr.setRequestHeader(name,value);
        const cancel=()=>xhr.abort(),finish=()=>signal.removeEventListener('abort',cancel);
        xhr.upload.onprogress=event=>progress?.(event.loaded,Boolean(body?.size&&event.loaded>=body.size));xhr.upload.onload=()=>progress?.(body?.size??0,true);
        xhr.onload=()=>{finish();if(xhr.status<200||xhr.status>=300){const header=xhr.getResponseHeader('Retry-After'),seconds=header===null?NaN:Number(header);reject(new UploadError(xhr.status,Number.isFinite(seconds)&&seconds>=0?Math.min(seconds*1000,5000):1000));return;}const value=xhr.response as UploadReply;if(!value||typeof value.upload_id!=='string'||!Number.isSafeInteger(value.offset)||!Number.isSafeInteger(value.size)||value.offset<0||value.offset>value.size||!['uploading','ready'].includes(value.state)||!Number.isFinite(Date.parse(value.expires_at))){reject(new UploadError(503));return;}resolve(value);};
        xhr.onerror=()=>{finish();reject(new UploadError(0));};xhr.onabort=()=>{finish();reject(new DOMException('Upload paused','AbortError'));};
        signal.addEventListener('abort',cancel,{once:true});if(signal.aborted){reject(new DOMException('Upload paused','AbortError'));return;}xhr.send(body??null);
    });
}
async function uploadRequest(method:string,path:string,body:Blob|undefined,headers:Record<string,string>,signal:AbortSignal,progress?:(bytes:number,finished:boolean)=>void):Promise<UploadReply>{
    for(let attempt=0;;attempt++){
        try{return await request(method,path,body,headers,signal,progress);}
        catch(error){
            if(!(error instanceof UploadError)||error.status!==429||attempt===2)throw error;
            await new Promise<void>((resolve,reject)=>{
                const done=()=>{clearTimeout(timer);signal.removeEventListener('abort',cancel);resolve();},cancel=()=>{clearTimeout(timer);reject(new DOMException('Upload paused','AbortError'));},timer=setTimeout(done,error.retryAfter);
                signal.addEventListener('abort',cancel,{once:true});if(signal.aborted)cancel();
            });
        }
    }
}
async function metadata(file:File,url:string,signal:AbortSignal):Promise<Partial<PendingAttachment>>{
    if(!/^(image|video)\//u.test(file.type))return {};
    return new Promise(resolve=>{
        const video=file.type.startsWith('video/'),element=video?document.createElement('video'):document.createElement('img');let done=false;
        const finish=(value:Partial<PendingAttachment>)=>{if(done)return;done=true;clearTimeout(timer);signal.removeEventListener('abort',cancel);if(element instanceof HTMLVideoElement){element.removeAttribute('src');element.load();}else element.src='';resolve(value);};
        const cancel=()=>finish({}),timer=setTimeout(cancel,3000);signal.addEventListener('abort',cancel,{once:true});element.onerror=cancel;
        const loaded=()=>{
            const width=element instanceof HTMLVideoElement?element.videoWidth:element.naturalWidth,height=element instanceof HTMLVideoElement?element.videoHeight:element.naturalHeight;
            if(!Number.isFinite(width)||!Number.isFinite(height)||width<=0||height<=0){finish({});return;}
            const result:Partial<PendingAttachment>={width,height};
            if(element instanceof HTMLVideoElement){if(Number.isFinite(element.duration)&&element.duration>0)result.duration_ms=Math.round(element.duration*1000);}
            else if(file.type==='image/jpeg')result.alpha=false;
            else if(width*height<=16000000){try{const canvas=document.createElement('canvas');canvas.width=Math.min(64,width);canvas.height=Math.min(64,height);const context=canvas.getContext('2d',{willReadFrequently:true});if(context){context.drawImage(element,0,0,canvas.width,canvas.height);const pixels=context.getImageData(0,0,canvas.width,canvas.height).data;for(let index=3;index<pixels.length;index+=4)if(pixels[index]<255){result.alpha=true;break;}}}catch{}}
            finish(result);
        };
        if(element instanceof HTMLVideoElement){element.preload='metadata';element.muted=true;element.onloadedmetadata=loaded;}else element.onload=loaded;
        if(signal.aborted)cancel();else element.src=url;
    });
}
export function createUploads(){
    const imports=new Map<string,AbortController>(),drafts=new Map<string,StagedFile[]>(),[revision,setRevision]=createSignal(0),[clock,setClock]=createSignal(Date.now()),[problem,setProblem]=createSignal('');
    const changed=()=>setRevision(value=>value+1),timer=setInterval(()=>setClock(Date.now()),1000);
    const files=(draft:string)=>{revision();return drafts.get(draft)??[];};
    const accept=(entry:StagedFile,value:UploadReply)=>{if(value.size!==entry.file.size||entry.remote&&value.upload_id!==entry.remote.upload_id||value.state==='ready'&&value.offset!==value.size)throw new UploadError(503);entry.remote=value;entry.bytes=value.offset;changed();};
    const start=async(entry:StagedFile)=>{
        entry.abort.abort();entry.abort=new AbortController();const signal=entry.abort.signal,token=++entry.generation;entry.phase='uploading';entry.problem='';changed();
        const preview=entry.metadata?Promise.resolve():metadata(entry.file,entry.attachment.url,signal).then(value=>{if(token===entry.generation){Object.assign(entry.attachment,value);entry.metadata=true;changed();}});
        try{
            if(entry.remote){try{accept(entry,await uploadRequest('GET','/web/v1/upload/'+encodeURIComponent(entry.remote.upload_id),undefined,{},signal));}catch(error){if(error instanceof UploadError&&[404,410].includes(error.status))entry.remote=undefined;else throw error;}}
            if(!entry.remote)accept(entry,await uploadRequest('POST','/web/v1/upload',new Blob(),{'Upload-Length':String(entry.file.size)},signal));
            while(entry.remote!.state!=='ready'){
                const offset=entry.remote!.offset,body=entry.file.slice(offset,Math.min(entry.file.size,offset+chunkSize));
                const result=await uploadRequest('POST','/web/v1/upload',body,{'Upload-ID':entry.remote!.upload_id,'Upload-Offset':String(offset),'Content-Type':'application/octet-stream'},signal,(bytes,finished)=>{if(token!==entry.generation)return;entry.bytes=Math.min(entry.file.size,offset+bytes);entry.phase=finished&&entry.bytes===entry.file.size?'finishing':'uploading';changed();});
                if(token!==entry.generation)return;accept(entry,result);
                if(result.state==='uploading'&&result.offset<=offset)throw new UploadError(503);
            }
            await preview;if(token!==entry.generation)return;entry.phase='ready';entry.problem='';changed();
        }catch(error){if(token!==entry.generation)return;entry.phase='paused';entry.problem=error instanceof DOMException&&error.name==='AbortError'?'Upload paused':error instanceof UploadError&&error.status===413?'File is too large':"Couldn't finish uploading. Try again.";changed();}
    };
    const add=(draft:string,selected:readonly File[])=>{
        const existing=drafts.get(draft)??[];
        if(existing.length+selected.length>10){setProblem('Choose up to 10 files');return;}
        if(selected.some(file=>file.size>maximum)){setProblem('Each file must be 100 MiB or smaller');return;}
        setProblem('');const added=selected.map(file=>{const key=crypto.randomUUID(),entry:StagedFile=createMutable({key,file,attachment:{key,name:file.name,type:file.type||'application/octet-stream',size:file.size,url:URL.createObjectURL(file)},phase:'uploading',bytes:0,problem:'',abort:new AbortController(),generation:0,metadata:false} as StagedFile);return entry;});
        drafts.set(draft,[...existing,...added]);changed();for(const entry of added)void start(entry);
    };
    const pause=(entry:StagedFile)=>{entry.generation++;entry.abort.abort();entry.phase='paused';entry.problem='Upload paused';changed();};
    const remove=(draft:string,key:string)=>{const entries=drafts.get(draft)??[],entry=entries.find(value=>value.key===key);if(!entry)return;entry.generation++;entry.abort.abort();URL.revokeObjectURL(entry.attachment.url);if(entry.remote)void fetch('/web/v1/upload/'+encodeURIComponent(entry.remote.upload_id),{method:'DELETE',credentials:'same-origin',cache:'no-store'}).catch(()=>{});drafts.set(draft,entries.filter(value=>value!==entry));changed();};
    const ready=(draft:string)=>{clock();const entries=files(draft);return entries.length>0&&entries.every(entry=>entry.phase==='ready'&&entry.remote?.state==='ready'&&Date.parse(entry.remote.expires_at)>Date.now());};
    const prepared=(draft:string):LocalMessageFile[]=>{if(!ready(draft))throw new Error('Attachments unavailable');return files(draft).map(entry=>({file:entry.file,attachment:{...entry.attachment},upload:{upload_id:entry.remote!.upload_id,name:entry.file.name},expires_at:entry.remote!.expires_at}));};
    const sent=(draft:string,submitted:readonly LocalMessageFile[])=>{if(!submitted.length)return;const keys=new Set(submitted.map(value=>value.attachment.key));drafts.set(draft,(drafts.get(draft)??[]).filter(entry=>!keys.has(entry.key)));changed();};
    const cancelImport=(draft:string)=>{imports.get(draft)?.abort();imports.delete(draft);changed();};
    const importAttachment=async(draft:string,attachment:BridgeRecord)=>{const url=attachmentOriginal(attachment);if(!url||Number(attachment.data.size)>maximum){setProblem("Original attachment is unavailable.");return;}cancelImport(draft);const abort=new AbortController();imports.set(draft,abort);changed();try{const response=await fetch(url,{credentials:"same-origin",cache:"no-store",signal:abort.signal});if(!response.ok||Number(response.headers.get("content-length"))>maximum)throw new Error();const blob=await response.blob();if(blob.size>maximum)throw new Error();if(!abort.signal.aborted)add(draft,[new File([blob],attachmentFilename(attachment),{type:String(attachment.data.mime_type??blob.type)})]);}catch{if(!abort.signal.aborted)setProblem("Couldn't prepare this attachment. Try forwarding it again.");}finally{if(imports.get(draft)===abort){imports.delete(draft);changed();}}};
    const busy=(draft:string)=>{revision();return imports.has(draft);};
    const dispose=()=>{for(const abort of imports.values())abort.abort();imports.clear();clearInterval(timer);for(const [draft,entries] of drafts)for(const entry of entries)remove(draft,entry.key);drafts.clear();};onCleanup(dispose);
    const expired=(entry:StagedFile)=>{clock();return Boolean(entry.remote&&Date.parse(entry.remote.expires_at)<=Date.now());};
    return {files,add,importAttachment,cancelImport,busy,pause,remove,retry:(entry:StagedFile)=>{void start(entry);},ready,expired,prepared,sent,problem,clearProblem:()=>setProblem(''),dispose};
}
export type Uploads=ReturnType<typeof createUploads>;
