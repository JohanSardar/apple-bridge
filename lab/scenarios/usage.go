package scenarios

import (
    "context"
    "encoding/json"
    "errors"
    "fmt"
    "math"
    "os"
    "os/exec"
    "path/filepath"
    "reflect"
    "strings"
    "time"
    "github.com/JohanSardar/apple-bridge/internal/contracts"
    "github.com/JohanSardar/apple-bridge/lab/world"
    "github.com/JohanSardar/apple-bridge/lab/scripted"
)

func runUsage(ctx context.Context,s Scenario,level string)(e Evidence) {
    sha,_:=exec.Command("git","rev-parse","HEAD").Output()
    e=Evidence{RunID:contracts.NewID("run"),Scenario:s.ID,Level:level,GitSHA:strings.TrimSpace(string(sha)),At:time.Now().UTC(),Assertions:[]Assertion{},Latency:map[string]Latency{},Hosts:[]string{"air"}}
    add:=func(name string,ok bool){e.Assertions=append(e.Assertions,Assertion{Name:name,Passed:ok})}
    defer func(){e.Passed=len(e.Assertions)>0;for _,a:=range e.Assertions {e.Passed=e.Passed&&a.Passed}}()
    if s.ID=="USE-SEGB-DECODE" {
        output,err:=exec.CommandContext(ctx,"packaging/dev/build-native.sh").CombinedOutput()
        if err!=nil {fmt.Fprintln(os.Stderr,string(output));add("decoder_build",false);return}
        path,err:=os.ReadFile("native/.build/native-output");if err!=nil {add("decoder_build",false);return}
        output,err=exec.CommandContext(ctx,filepath.Join(strings.TrimSpace(string(path)),"UsageDecoderChecks"),"lab/fixtures/usage").CombinedOutput()
        if err!=nil {fmt.Fprintln(os.Stderr,string(output))}
        add("scrubbed_segb_and_session_math",err==nil);return
    }
    ctx=world.WithOptions(ctx,world.Options{Scope:"usage:read,reminders:read,system:read"})
    var w *world.World
    var err error
    if level=="real" {w,err=world.NewReal(ctx,level,0)} else {w,err=world.New(ctx,level,1,0)}
    if err!=nil {fmt.Fprintln(os.Stderr,"usage world:",err);add("world_start",false);return}
    defer w.Close()
    if level=="real" {
        response,err:=w.Truth("observe",map[string]any{"source":"usage"})
        if err!=nil {fmt.Fprintln(os.Stderr,"usage oracle unavailable");add("independent_decode",false);return}
        raw,_:=json.Marshal(response["records"])
        var observations []contracts.Observation
        if json.Unmarshal(raw,&observations)!=nil {add("independent_decode",false);return}
        compare:=func()bool {
            aliases,err:=gatewayAliases(w);if err!=nil{return false}
            expected,err:=truthRecords(observations,aliases);if err!=nil{return false}
            actual,err:=w.Records(w.Gateway);if err!=nil{return false}
            indexed:=map[string]contracts.Record{}
            for _,r:=range actual {if r.Source==contracts.Usage && !r.Deleted {indexed[r.Collection+"/"+r.ID]=r}}
            if len(indexed)!=len(expected){return false}
            for _,native:=range expected {
                record,ok:=indexed[native.Collection+"/"+native.ID];if !ok{return false}
                var a,b map[string]any;json.Unmarshal(record.Data,&a);json.Unmarshal(native.Data,&b)
                for _,field:=range []string{"seconds_estimated","clipped_seconds"} {
                    av,aok:=a[field].(float64);bv,bok:=b[field].(float64)
                    if aok!=bok || (aok && math.Abs(av-bv)>0.002){return false}
                    delete(a,field);delete(b,field)
                }
                if a["latest_event_at"]!=b["latest_event_at"] {
                    av,aok:=a["latest_event_at"].(string);bv,bok:=b["latest_event_at"].(string)
                    at,ae:=time.Parse(time.RFC3339Nano,av);bt,be:=time.Parse(time.RFC3339Nano,bv)
                    if !aok || !bok || ae!=nil || be!=nil || math.Abs(at.Sub(bt).Seconds())>0.002 {return false}
                }
                delete(a,"latest_event_at");delete(b,"latest_event_at")
                if !reflect.DeepEqual(a,b){return false}
            }
            return true
        }
        add("devices_and_estimates_match_independent_decode",w.Await(15*time.Second,compare)==nil)
        return
    }
    base:=float64(time.Date(2026,10,16,12,0,0,0,time.UTC).Unix())
    event:=func(offset float64,start bool,bundle string)map[string]any{return map[string]any{"at":base+offset,"starting":start,"bundle":bundle}}
    fixture:=func(id,platform string,local bool,events ...map[string]any)error {
        return w.ControlAction("/usage",map[string]any{"id":id,"name":"synthetic-device","platform":platform,"is_local":local,"events":events})
    }
    rows:=func(collection string)[]contracts.Record {
        all,_:=w.Records(w.Replicas[0]);filtered:=[]contracts.Record{}
        for _,r:=range all {if r.Collection==collection && !r.Deleted {filtered=append(filtered,r)}}
        return filtered
    }
    if s.ID=="USE-READONLY" {
        client:=scripted.Client{Endpoint:w.Gateway,Protocol:"rest"}
        for _,operation:=range []string{"usage.create","usage.update","usage.delete","usage.create_device","usage.app_day.update","usage.write"} {
            err:=client.Call(ctx,operation,map[string]any{"args":map[string]any{}},nil)
            add(operation+"_unsupported",errors.Is(err,contracts.ErrUnsupported))
        }
        effects,err:=w.Effects();add("no_usage_effect_dispatched",err==nil && len(effects)==0);return
    }
    if s.ID=="USE-LOCK-CLIP" {
        err=fixture("synthetic-mac","mac",true,event(0,true,"test.app"),event(60,true,"loginwindow"),event(600,true,"test.app"),event(660,false,"test.app"),event(7200,true,"test.app"),event(30000,false,"test.app"))
        if err!=nil {add("fixture",false);return}
        add("lock_focus_excluded_and_four_hour_cap",w.Await(5*time.Second,func()bool{
            records:=rows("usage.app_day");if len(records)!=1{return false};var data map[string]any;json.Unmarshal(records[0].Data,&data)
            return data["seconds_estimated"]==float64(14520) && data["clipped_seconds"]==float64(540) && data["sessions"]==float64(3) && data["bundle_id"]=="test.app"
        })==nil)
    } else if s.ID=="USE-REMOTE-LAG" {
        err=fixture("synthetic-phone","iphone",false,event(0,true,"test.new"),event(120,false,"test.new"))
        if err!=nil {add("initial_batch",false);return}
        if w.Await(5*time.Second,func()bool{return len(rows("usage.app_day"))==1})!=nil {add("initial_batch_visible",false);return}
        first:=rows("usage.device");if len(first)!=1 {add("device",false);return}
        var before map[string]any;json.Unmarshal(first[0].Data,&before)
        err=fixture("synthetic-phone","iphone",false,event(-86400,true,"test.old"),event(-86280,false,"test.old"))
        add("delayed_past_day_batch_applied",err==nil && w.Await(5*time.Second,func()bool{return len(rows("usage.app_day"))==2})==nil)
        devices:=rows("usage.device");var after map[string]any;if len(devices)==1 {json.Unmarshal(devices[0].Data,&after)}
        add("freshness_is_newest_event_per_device",before["latest_event_at"]==after["latest_event_at"])
    } else {
        err=fixture("synthetic-mac","mac",true,event(0,true,"test.app"),event(120,false,"test.app"))
        if err==nil {err=fixture("synthetic-phone","iphone",false,event(-86400,true,"test.app"),event(-86160,false,"test.app"))}
        if err!=nil {add("local_remote_fixtures",false);return}
        add("local_remote_devices_and_days",w.Await(5*time.Second,func()bool{return len(rows("usage.device"))==2 && len(rows("usage.app_day"))==2})==nil)
        devices:=rows("usage.device");localID:=""
        for _,record:=range devices {var data map[string]any;json.Unmarshal(record.Data,&data);if data["is_local"]==true {localID=record.ID}}
        for _,protocol:=range []string{"rest","mcp"} {
            client:=scripted.Client{Endpoint:w.Replicas[0],Protocol:protocol}
            var result contracts.ReadResult
            err=client.Call(ctx,"usage.summary",contracts.UsageQuery{DeviceID:localID,FromDay:"2026-10-16",ThroughDay:"2026-10-16",BundleID:"test.app"},&result)
            add(protocol+"_typed_local_summary",err==nil && len(result.Records)==1)
        }
    }
    add("truth_gateway_replica_converge",w.Await(5*time.Second,func()bool{return convergence(w)})==nil)
    return
}
