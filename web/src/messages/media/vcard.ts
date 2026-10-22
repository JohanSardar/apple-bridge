export type ContactField={label:string;value:string};
export type VCard={name:string;fields:ContactField[];photo?:{type:'image/png'|'image/jpeg';bytes:Uint8Array}};
function unescape(value:string){return value.replace(/\\([nN,;\\])/gu,(_,letter:string)=>letter.toLowerCase()==='n'?'\n':letter).trim().slice(0,1024);}
function components(value:string){const result:string[]=[];let field='',escaped=false;for(const character of value){if(character===';'&&!escaped){result.push(field);field='';}else field+=character;escaped=character==='\\'?!escaped:false;}result.push(field);return result;}
function picture(value:string):VCard['photo']{
 const encoded=value.replace(/^data:image\/(png|jpeg);base64,/iu,'').replace(/\s/gu,'');if(encoded.length>80000||!/^[a-z0-9+/]*={0,2}$/iu.test(encoded))return;
 try{const bytes=Uint8Array.from(atob(encoded),character=>character.charCodeAt(0));if(bytes.length>60000)return;if(bytes[0]===137&&bytes[1]===80&&bytes[2]===78&&bytes[3]===71&&bytes[4]===13&&bytes[5]===10&&bytes[6]===26&&bytes[7]===10)return {type:'image/png',bytes};if(bytes[0]===255&&bytes[1]===216&&bytes[2]===255)return {type:'image/jpeg',bytes};}catch{}return;
}
export function decodeVCard(source:string):VCard {
 if(source.length>65536)throw new Error('Contact is too large');
 const lines=source.replace(/\r\n[ \t]/gu,'').replace(/\n[ \t]/gu,'').split(/\r?\n/u);if(!lines.some(line=>line.toUpperCase()==='BEGIN:VCARD')||!lines.some(line=>line.toUpperCase()==='END:VCARD'))throw new Error('Contact format unavailable');
 const version=lines.find(line=>line.toUpperCase().startsWith('VERSION:'));if(version&&!/^VERSION:(3\.0|4\.0)$/iu.test(version))throw new Error('Contact version unavailable');
 const contact:VCard={name:'Contact Card',fields:[]};let structured='';
 for(const line of lines){const separator=line.indexOf(':');if(separator<0)continue;const header=line.slice(0,separator).split(';'),key=header[0].split('.').at(-1)!.toUpperCase(),raw=line.slice(separator+1),value=unescape(raw);if(!value)continue;
  if(key==='FN')contact.name=value;
  else if(key==='N')structured=[3,1,2,0,4].map(index=>unescape(components(raw)[index]??'')).filter(Boolean).join(' ');
  else if(key==='PHOTO')contact.photo=picture(raw);
  else if(['TEL','EMAIL','ORG','TITLE','ADR','URL','NOTE','BDAY'].includes(key)&&contact.fields.length<32){const label:Record<string,string>={TEL:'Phone',EMAIL:'Email',ORG:'Company',TITLE:'Job title',ADR:'Address',URL:'Website',NOTE:'Note',BDAY:'Birthday'};contact.fields.push({label:label[key],value:key==='ADR'||key==='ORG'?components(raw).map(unescape).filter(Boolean).join(', '):value});}
 }
 if(contact.name==='Contact Card'&&structured)contact.name=structured;
 return contact;
}
export async function readVCard(source:string,signal:AbortSignal):Promise<VCard>{
 if(!source.startsWith('/web/v1/blob/')||new URL(source,location.origin).origin!==location.origin)throw new Error('Contact source unavailable');
 const response=await fetch(source,{signal,credentials:'same-origin'});if(!response.ok)throw new Error('Contact unavailable');if(Number(response.headers.get('Content-Length'))>65536)throw new Error('Contact is too large');
 const reader=response.body?.getReader();if(!reader)throw new Error('Contact unavailable');const chunks:Uint8Array[]=[];let size=0;
 try{while(true){const next=await reader.read();if(next.done)break;size+=next.value.length;if(size>65536)throw new Error('Contact is too large');chunks.push(next.value);}}catch(error){await reader.cancel().catch(()=>{});throw error;}finally{reader.releaseLock();}
 const bytes=new Uint8Array(size);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}return decodeVCard(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
}
