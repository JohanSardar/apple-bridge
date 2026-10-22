const pending=new Map<string,Promise<unknown>>();
export function loadCatalog<T>(url:string):Promise<T>{
    let value=pending.get(url);
    if(!value){value=fetch(url).then(response=>{if(!response.ok)throw new Error('Catalog unavailable');return response.json();}).catch(error=>{pending.delete(url);throw error;});pending.set(url,value);}
    return value as Promise<T>;
}
