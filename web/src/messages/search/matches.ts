export function matchRanges(value:string,query:string){
    const fold=(text:string)=>text.normalize('NFD').replace(/\p{M}/gu,'').toLocaleLowerCase();
    const terms=(query.match(/[\p{L}\p{N}\p{M}]+/gu)??[]).map(fold).filter(Boolean),starts:number[]=[],ends:number[]=[];let lower='',position=0;
    if(!terms.length)return [];
    for(const character of value){const folded=fold(character);for(let index=0;index<folded.length;index++){starts.push(position);ends.push(position+character.length);}lower+=folded;position+=character.length;}
    const ranges:{start:number;end:number}[]=[];let offset=0;
    while(offset<lower.length){let start=-1,length=0;for(const term of terms){const index=lower.indexOf(term,offset);if(index>=0&&(start<0||index<start||index===start&&term.length>length)){start=index;length=term.length;}}if(start<0)break;ranges.push({start:starts[start],end:ends[start+length-1]});offset=start+length;}
    return ranges;
}
