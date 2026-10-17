package simapple

import (
    "encoding/json"
    "net/http"
    "sort"
    "time"
    "math"
    "github.com/JohanSardar/apple-bridge/internal/contracts"
)

type usageEvent struct { At float64 `json:"at"`; Starting bool `json:"starting"`; Bundle string `json:"bundle"` }
type usageDevice struct { ID string `json:"id"`; Name string `json:"name"`; Platform string `json:"platform"`; Local bool `json:"is_local"`; Events []usageEvent `json:"events"` }
type usageTotal struct { seconds,clipped,latest float64; sessions int; day,bundle string }

func (f *fakeApple) usageRoutes(mux *http.ServeMux) {
    mux.HandleFunc("POST /usage",func(w http.ResponseWriter,r *http.Request){
        var device usageDevice
        if json.NewDecoder(http.MaxBytesReader(w,r.Body,contracts.MaxFrame)).Decode(&device)!=nil || device.ID=="" || (device.Platform!="mac" && device.Platform!="iphone" && device.Platform!="ipad") {http.Error(w,"invalid",400);return}
        f.mu.Lock()
        if f.usage==nil {f.usage=map[string]usageDevice{}}
        previous:=f.usage[device.ID]
        device.Events=append(previous.Events,device.Events...)
        sort.Slice(device.Events,func(i,j int)bool {a,b:=device.Events[i],device.Events[j];if a.At!=b.At{return a.At<b.At};if a.Starting!=b.Starting{return !a.Starting};return a.Bundle<b.Bundle})
        unique:=[]usageEvent{}
        for _,event:=range device.Events {if len(unique)==0 || unique[len(unique)-1]!=event {unique=append(unique,event)}}
        device.Events=unique;f.usage[device.ID]=device
        observations:=usageObservations(device)
        for _,o:=range observations {f.records[o.NativeKey]=o}
        err:=f.changedLocked(contracts.Usage);revision:=f.durable.Revision[contracts.Usage]
        f.mu.Unlock()
        if err!=nil {http.Error(w,"persist",500);return}
        if err=f.enqueue(r.Context(),fullScanSource(observations,contracts.Usage),true,revision);err!=nil {http.Error(w,"capture",500);return}
        json.NewEncoder(w).Encode(map[string]bool{"committed":true})
    })
}
func usageObservations(device usageDevice) []contracts.Observation {
    observations:=[]contracts.Observation{}
    stamp:=func(value float64)string {return time.Unix(0,int64(value*1e9)).UTC().Format("2006-01-02T15:04:05.000Z")}
    emit:=func(collection,key string,data map[string]any){raw,_:=json.Marshal(data);observations=append(observations,contracts.Observation{Kind:"upsert",Collection:collection,NativeKey:key,Data:raw,ObservedAt:time.Now().UTC()})}
    latest:="";if len(device.Events)>0 {latest=stamp(device.Events[len(device.Events)-1].At)}
    emit("usage.device",device.ID,map[string]any{"name":device.Name,"platform":device.Platform,"build":"sim","is_local":device.Local,"latest_event_at":latest})
    totals:=map[string]usageTotal{}
    var current *usageEvent
    previous:=""
    allocate:=func(start,end float64,bundle string,locked bool){
        latest:=end;end=math.Min(end,start+14400)
        for cursor:=start;cursor<end; {
            date:=time.Unix(0,int64(cursor*1e9)).UTC();day:=date.Format("2006-01-02")
            midnight:=time.Date(date.Year(),date.Month(),date.Day()+1,0,0,0,0,time.UTC)
            through:=math.Min(end,float64(midnight.Unix()))
            if through<=cursor {break}
            key:=day+"/"+bundle;value:=totals[key];value.day=day;value.bundle=bundle;value.latest=math.Max(value.latest,latest)
            if locked {value.clipped+=through-cursor} else {value.seconds+=through-cursor;value.sessions++}
            totals[key]=value;cursor=through
        }
    }
    close:=func(at float64){
        if current==nil || at<=current.At{return}
        locked:=device.Platform=="mac" && (current.Bundle=="loginwindow" || current.Bundle=="com.apple.loginwindow" || current.Bundle=="com.apple.ScreenSaver.Engine")
        if locked {if previous!="" {allocate(current.At,at,previous,true)}} else {allocate(current.At,at,current.Bundle,false);previous=current.Bundle}
    }
    for i:=range device.Events {event:=device.Events[i];if event.Starting {close(event.At);current=&device.Events[i]} else if current!=nil && current.Bundle==event.Bundle {close(event.At);current=nil}}
    for _,value:=range totals {
        emit("usage.app_day",device.ID+"/"+value.day+"/"+value.bundle,map[string]any{"device_id":map[string]any{"$ref":map[string]string{"collection":"usage.device","native_key":device.ID}},"day":value.day,"time_zone":"UTC","bundle_id":value.bundle,"seconds_estimated":value.seconds,"sessions":value.sessions,"clipped_seconds":value.clipped,"provenance":"biome.app_infocus","decoder_version":"infocus-1","latest_event_at":stamp(value.latest),"uncertainty":"Synthetic focus estimates; delayed batches supported; settings parity unproven"})
    }
    return observations
}
