package scenarios

import (
	"context"
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func runSnapshotPool(ctx context.Context, w *world.World, evidence *Evidence, add func(string, bool)) {
	add("paired_replica", w.AddReplicaScope("admin") == nil)
	if len(w.Replicas) == 0 {
		return
	}
	authority, err := trust.Open(w.Replicas[0].Home)
	if err != nil {
		add("replica_identity", false)
		return
	}
	enrollment, err := authority.Enrollment()
	if err != nil || enrollment == nil {
		add("replica_enrollment", false)
		return
	}
	transport := &http.Transport{TLSClientConfig: authority.ClientTLS(enrollment.Fingerprint)}
	defer transport.CloseIdleConnections()
	bounded, cancel := context.WithTimeout(ctx, 12*time.Second)
	defer cancel()
	type paused struct {
		socket *websocket.Conn
		bytes  int
		begin  contracts.FeedFrame
		cursor contracts.Cursor
	}
	var feeds []paused
	defer func() {
		for _, feed := range feeds {
			feed.socket.CloseNow()
		}
	}()
	started := time.Now()
	for len(feeds) < 12 {
		socket, _, err := websocket.Dial(bounded, "wss://"+enrollment.Address+"/v1/feed", &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
		if err != nil {
			add("twelve_paused_snapshots_with_pool_8", false)
			return
		}
		entry := paused{socket: socket}
		feeds = append(feeds, entry)
		err = wsjson.Write(bounded, socket, contracts.FeedFrame{T: "Hello", Protocol: contracts.FeedProtocol, ReplicaID: enrollment.ReplicaID, SchemaOffer: []int{1}, Credits: contracts.ByteCredit})
		if err != nil {
			add("snapshot_hello", false)
			return
		}
		for {
			_, raw, problem := socket.Read(bounded)
			if problem != nil {
				add("twelve_paused_snapshots_with_pool_8", false)
				return
			}
			var frame contracts.FeedFrame
			if json.Unmarshal(raw, &frame) != nil {
				add("snapshot_frame", false)
				return
			}
			if frame.T == "Heartbeat" {
				continue
			}
			if frame.T == "Welcome" {
				entry.cursor = contracts.Cursor{Epoch: frame.Epoch, SubscriptionRevision: frame.SubscriptionRevision}
			}
			if frame.T == "SnapshotBegin" {
				entry.begin, entry.bytes, entry.cursor.Sequence = frame, len(raw), frame.S
				feeds[len(feeds)-1] = entry
				break
			}
			if wsjson.Write(bounded, socket, contracts.FeedFrame{T: "Ack", Credits: len(raw)}) != nil {
				add("snapshot_ack", false)
				return
			}
		}
	}
	add("twelve_paused_snapshots_with_pool_8", true)
	evidence.Latency["twelve_snapshot_begins"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	var sources contracts.ReadResult
	err = (scripted.Client{Endpoint: w.Gateway}).Call(bounded, "sources.status", struct{}{}, &sources)
	add("reads_progress_with_paused_snapshots", err == nil && len(sources.Records) > 0)
	add("paused_source_change_queued", w.ControlAction("/messages", map[string]any{"action": "capability-unproven"}) == nil)
	add("writes_progress_with_paused_snapshots", w.Await(3*time.Second, func() bool {
		if (scripted.Client{Endpoint: w.Gateway}).Call(bounded, "sources.status", struct{}{}, &sources) != nil {
			return false
		}
		for _, record := range sources.Records {
			var source struct {
				Reason string `json:"reason"`
			}
			if record.Source == contracts.Messages && json.Unmarshal(record.Data, &source) == nil && source.Reason == "capability_unproven" {
				return true
			}
		}
		return false
	}) == nil)
	for _, feed := range feeds {
		if wsjson.Write(bounded, feed.socket, contracts.FeedFrame{T: "Ack", Credits: feed.bytes}) != nil {
			add("paused_snapshot_resumes", false)
			return
		}
		var records []contracts.Record
		for {
			_, raw, problem := feed.socket.Read(bounded)
			if problem != nil {
				add("paused_snapshot_resumes", false)
				return
			}
			var frame contracts.FeedFrame
			if json.Unmarshal(raw, &frame) != nil {
				add("resumed_snapshot_frame", false)
				return
			}
			if frame.T == "Heartbeat" {
				continue
			}
			ack := contracts.FeedFrame{T: "Ack", Credits: len(raw)}
			if frame.T == "SnapshotPage" {
				records = append(records, frame.Records...)
			}
			if frame.T == "SnapshotEnd" {
				ack.Cursor = &feed.cursor
			}
			if wsjson.Write(bounded, feed.socket, ack) != nil {
				add("resumed_snapshot_ack", false)
				return
			}
			if frame.T == "SnapshotEnd" {
				add("materialized_snapshot_stays_frozen", len(records) == feed.begin.Count && state.Digest(records) == feed.begin.Digest)
				break
			}
		}
	}
	add("paused_snapshot_resumes", true)
}
