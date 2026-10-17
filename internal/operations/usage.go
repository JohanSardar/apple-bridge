package operations

import (
    "context"
    "encoding/json"
    "time"
    "github.com/JohanSardar/apple-bridge/internal/contracts"
    "github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) usageSummary(ctx context.Context,p trust.Principal,q contracts.UsageQuery)(contracts.ReadResult,error) {
    if q.Limit<0 || q.Limit>10000 || (q.FromDay!="" && q.ThroughDay!="" && q.FromDay>q.ThroughDay) {return contracts.ReadResult{},contracts.ErrInvalid}
    for _,day:=range []string{q.FromDay,q.ThroughDay} {if day!="" {if parsed,err:=time.Parse("2006-01-02",day);err!=nil || parsed.Format("2006-01-02")!=day {return contracts.ReadResult{},contracts.ErrInvalid}}}
    limit:=q.Limit;if limit==0 {limit=100}
    result:=contracts.ReadResult{Records:[]contracts.Record{},Cursor:s.State.Head()}
    after:=q.After
    for {
        page,err:=s.State.Read(ctx,p.Visibility(),contracts.ReadQuery{Collection:"usage.app_day",After:after,Limit:1000})
        if err!=nil {return result,err}
        result.Cursor=page.Cursor
        for _,record:=range page.Records {
            var data struct {DeviceID string `json:"device_id"`; Day string `json:"day"`; BundleID string `json:"bundle_id"`}
            if json.Unmarshal(record.Data,&data)!=nil {return result,contracts.ErrCorrupt}
            if (q.DeviceID=="" || q.DeviceID==data.DeviceID) && (q.FromDay=="" || data.Day>=q.FromDay) && (q.ThroughDay=="" || data.Day<=q.ThroughDay) && (q.BundleID=="" || q.BundleID==data.BundleID) {
                if len(result.Records)==limit {return result,nil}
                result.Records=append(result.Records,record)
            }
            after=record.Collection+"/"+record.ID
            if len(result.Records)==limit {result.Next=after;return result,nil}
        }
        if page.Next=="" {return result,nil}
        after=page.Next
    }
}
