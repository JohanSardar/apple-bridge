package replication

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"net/http"
	"slices"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
)

type Server struct {
	UploadControl func(context.Context, *x509.Certificate, string, string, json.RawMessage) (contracts.CommandResponse, error)
	UploadChunk   func(context.Context, *x509.Certificate, string, []byte) (string, contracts.CommandResponse, error)
	UploadClosed  func(string)

	delegations map[string]*delegationSession
	mu          sync.Mutex
	statuses    map[string]contracts.DialStatus
	Command     func(context.Context, *x509.Certificate, string, json.RawMessage) (contracts.CommandResponse, error)
	Blobs       *blobs.Store

	Store     *state.Store
	Authority *trust.Authority
}

func read(ctx context.Context, conn *websocket.Conn) (contracts.FeedFrame, int, error) {
	var f contracts.FeedFrame
	bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	kind, b, err := conn.Read(bounded)
	if err != nil {
		return f, 0, err
	}
	if kind != websocket.MessageText || len(b) > contracts.MaxFrame {
		return f, len(b), contracts.ErrInvalid
	}
	err = json.Unmarshal(b, &f)
	if err == nil && f.T == "" {
		err = contracts.ErrInvalid
	}
	return f, len(b), err
}
func send(ctx context.Context, conn *websocket.Conn, f contracts.FeedFrame) (int, error) {
	b, err := json.Marshal(f)
	if err != nil {
		return 0, err
	}
	if len(b) > contracts.MaxFrame {
		return 0, contracts.ErrCapacity
	}
	bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	return len(b), writeFrame(bounded, conn, websocket.MessageText, b, false)
}

func (s *Server) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.TLS == nil || len(r.TLS.PeerCertificates) != 1 || r.Header.Get("Origin") != "" {
			http.Error(w, "unauthorized", 401)
			return
		}
		p, err := s.Authority.AuthenticateCertificate(r.TLS.PeerCertificates[0])
		if err != nil {
			http.Error(w, "unauthorized", 401)
			return
		}
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		conn.SetReadLimit(contracts.MaxFrame)
		ctx, cancel := s.Authority.WatchPrincipal(r.Context(), p)
		defer cancel()
		s.serve(ctx, conn, p, r.TLS.PeerCertificates[0])
	})
}

func (s *Server) serve(ctx context.Context, conn *websocket.Conn, p trust.Principal, cert *x509.Certificate) error {
	conn.SetReadLimit(contracts.MaxFrame)
	hello, _, err := read(ctx, conn)
	if err != nil {
		return err
	}
	if hello.T != "Hello" || hello.Protocol != contracts.FeedProtocol || hello.ReplicaID != p.ID || !slices.Contains(hello.SchemaOffer, 1) || hello.Credits <= 0 {
		return contracts.ErrInvalid
	}
	if p.Dial != "" {
		s.Authority.ConfirmDial(p.ID)
	}
	clockProtocol := ""
	if hello.ClockProtocol == contracts.FeedClockProtocol {
		clockProtocol = contracts.FeedClockProtocol
	}
	negotiatedDelegation := ""
	if hello.DelegationProtocol == delegationProtocol {
		negotiatedDelegation = delegationProtocol
	}
	s.updateDial(p.ID, func(status *contracts.DialStatus) {
		status.State = "connected"
		status.LastHeartbeat = time.Now().UTC()
	})
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	ctx = priorityWriter(ctx, conn)
	sessionID := contracts.NewID("feed")
	defer func() {
		cancel()
		if s.UploadClosed != nil {
			s.UploadClosed(sessionID)
		}
	}()
	conn.SetReadLimit(contracts.BlobChunkSize + 64)
	wants := make(chan contracts.FeedFrame, 8)
	go s.serveBlobs(ctx, conn, p, wants)
	commandSlots := make(chan struct{}, 16)
	forward := func(f contracts.FeedFrame) {
		defer func() { <-commandSlots }()
		bounded, stop := context.WithTimeout(ctx, 20*time.Second)
		defer stop()
		var response contracts.CommandResponse
		var err error
		if f.T == "UploadControl" {
			response, err = s.UploadControl(bounded, cert, sessionID, f.Delegation, f.Request)
		} else {
			response, err = s.Command(bounded, cert, f.Delegation, f.Request)
		}
		if err != nil {
			cancel()
			return
		}
		if _, err = send(ctx, conn, contracts.FeedFrame{T: "CommandResponse", ID: f.ID, Response: &response}); err != nil {
			cancel()
		}
	}

	window := newFeedWindow(min(hello.Credits, contracts.ByteCredit))
	defer window.close()
	go func() {
		defer cancel()
		for {
			f, err := s.readUpstream(ctx, conn, cert, sessionID)
			if err != nil {
				return
			}
			if p.Dial != "" {
				s.updateDial(p.ID, func(status *contracts.DialStatus) {
					status.LastHeartbeat = time.Now().UTC()
					if f.Cursor != nil {
						status.Cursor = *f.Cursor
					}
				})
			}
			received := time.Now()
			switch f.T {
			case "ClockPing":
				if clockProtocol == "" || clockPong(ctx, conn, f, received) != nil {
					return
				}
			case "ClockAck":
				if clockProtocol == "" || f.Clock == nil {
					return
				}
				if _, _, err := f.Clock.Estimate(); err != nil {
					return
				}

			case "DelegationResponse":
				if negotiatedDelegation == "" || !s.deliverDelegation(p.ID, conn, f) {
					return
				}
			case "CommandRequest", "UploadControl":
				if f.ID == "" || len(f.Request) == 0 || f.T == "CommandRequest" && s.Command == nil || f.T == "UploadControl" && s.UploadControl == nil {
					return
				}
				select {
				case commandSlots <- struct{}{}:
					go forward(f)
				default:
					return
				}

			case "Ack":
				if window.acknowledge(f) != nil {
					return
				}
			case "BlobWant":
				select {
				case wants <- f:
				case <-ctx.Done():
					return
				}
			case "Heartbeat":
			default:
				return
			}
		}
	}()
	go func() {
		tick := time.NewTicker(5 * time.Second)
		defer tick.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-tick.C:
				if _, err := send(ctx, conn, contracts.FeedFrame{T: "Heartbeat", Head: s.Store.Head().Sequence, At: time.Now().UTC()}); err != nil {
					cancel()
					return
				}
			}
		}
	}()
	transmit := func(f contracts.FeedFrame, expected *contracts.Cursor) error {
		raw, err := json.Marshal(f)
		if err != nil {
			return err
		}
		receipt, err := window.reserve(ctx, cancel, len(raw), expected)
		if err != nil {
			return err
		}
		if _, err = send(ctx, conn, f); err != nil {
			return err
		}
		if f.T == "Batch" {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-receipt.done:
			return nil
		}
	}
	head := s.Store.Head()
	head.SubscriptionRevision = p.Revision
	resume := hello.Cursor != nil && hello.Cursor.Epoch == head.Epoch && hello.Cursor.SubscriptionRevision == p.Revision && hello.Cursor.Sequence <= head.Sequence && hello.Cursor.Sequence >= s.Store.RetentionFloor()
	reason := "snapshot"
	if resume {
		reason = "resume"
	}
	features := []string(nil)
	if slices.Contains(hello.Features, "capture_atomic") {
		features = []string{"capture_atomic"}
	}
	if err = transmit(contracts.FeedFrame{T: "Welcome", Features: features, DelegationProtocol: negotiatedDelegation, ClockProtocol: clockProtocol, At: time.Now().UTC(), Protocol: contracts.FeedProtocol, GatewayID: s.Authority.Fingerprint, Epoch: head.Epoch, SubscriptionRevision: p.Revision, Scope: p.Grants, Head: head.Sequence, RetentionFloor: s.Store.RetentionFloor(), Schema: 1, Reason: reason}, nil); err != nil {
		return err
	}
	if negotiatedDelegation != "" {
		unregister := s.registerDelegation(ctx, conn, p, cert)
		defer unregister()
	}
	cursor := head
	if resume {
		cursor = *hello.Cursor
	} else {
		if hello.Cursor != nil && hello.Cursor.Sequence > 0 {
			if err = transmit(contracts.FeedFrame{T: "Reset", Reason: "history_or_scope_changed", RetentionFloor: s.Store.RetentionFloor()}, nil); err != nil {
				return err
			}
		}
		snapshot, err := s.Store.FreezeSnapshot(ctx, p.Visibility(), p.Revision)
		if err != nil {
			return err
		}
		defer snapshot.Close()
		cursor = snapshot.Cursor
		if err = transmit(contracts.FeedFrame{T: "SnapshotBegin", ID: snapshot.ID, S: cursor.Sequence, Count: snapshot.Count, Digest: snapshot.Digest, ExpiresAt: snapshot.ExpiresAt, Scope: p.Grants}, nil); err != nil {
			return err
		}
		ordinal := 0
		for offset := 0; offset < len(snapshot.Records); {
			end := min(offset+64, len(snapshot.Records))
			page := contracts.FeedFrame{T: "SnapshotPage", ID: snapshot.ID, Ordinal: ordinal, Records: snapshot.Records[offset:end]}
			for {
				b, _ := json.Marshal(page)
				if len(b) <= contracts.MaxFrame {
					break
				}
				end--
				if end == offset {
					return contracts.ErrCapacity
				}
				page.Records = snapshot.Records[offset:end]
			}
			if err = snapshot.Renew(ctx); err != nil {
				return err
			}
			if err = transmit(page, nil); err != nil {
				return err
			}
			offset = end
			ordinal++
		}
		if err = transmit(contracts.FeedFrame{T: "SnapshotEnd", ID: snapshot.ID}, &cursor); err != nil {
			return err
		}
		snapshot.Close()
	}
	journalCursor := cursor
	journalCursor.SubscriptionRevision = s.Store.Head().SubscriptionRevision
	changes := s.Store.FeedChanges
	if len(features) == 0 {
		changes = s.Store.LegacyChanges
	}
	iterator, err := changes(ctx, p.Visibility(), journalCursor)
	if err != nil {
		return err
	}
	for {
		batch, err := iterator.Next(ctx)
		if err != nil {
			if errors.Is(err, contracts.ErrReset) {
				send(ctx, conn, contracts.FeedFrame{T: "Reset", Reason: "retention", RetentionFloor: s.Store.RetentionFloor()})
			}
			return err
		}
		batch.SubscriptionRevision = p.Revision
		cursor.Sequence = batch.Through
		expected := &cursor
		if batch.Partial {
			expected = nil
		}
		if err = transmit(contracts.FeedFrame{T: "Batch", Batch: &batch}, expected); err != nil {
			return err
		}
	}
}

type FollowStatus = contracts.ReplicaStatus

type Client struct {
	session      *commandSession
	connectionMu sync.Mutex
	GatewayClock func(contracts.ClockSample)

	Blobs *blobs.Store

	Store     *state.Store
	Authority *trust.Authority
	mu        sync.Mutex
	status    FollowStatus
}

func (c *Client) Status() FollowStatus {
	c.mu.Lock()
	defer c.mu.Unlock()
	s := c.status
	s.Cursor = c.Store.Head()
	if c.Blobs != nil {
		s.Blobs = c.Blobs.Progress()
	}
	s.Role = "replica"
	if s.GatewayHead >= s.Cursor.Sequence {
		s.Lag = s.GatewayHead - s.Cursor.Sequence
	}
	if !s.LastHeartbeat.IsZero() {
		s.StalenessMS = max(0, time.Since(s.LastHeartbeat).Milliseconds())
	}
	if !s.LastHeartbeat.IsZero() && time.Since(s.LastHeartbeat) > 15*time.Second {
		s.State = "disconnected"
	}
	return s
}
func (c *Client) update(fn func(*FollowStatus)) { c.mu.Lock(); fn(&c.status); c.mu.Unlock() }
func (c *Client) Follow(ctx context.Context) error {
	enrollment, err := c.Authority.Enrollment()
	if err != nil {
		return err
	}
	if enrollment == nil {
		return contracts.ErrUnauthorized
	}
	for ctx.Err() == nil {
		c.update(func(s *FollowStatus) { s.State = "connecting" })
		c.follow(ctx, *enrollment)
		c.update(func(s *FollowStatus) { s.State = "disconnected" })
		timer := time.NewTimer(time.Second)
		select {
		case <-ctx.Done():
			timer.Stop()
			return nil
		case <-timer.C:
		}
	}
	return nil
}
func (c *Client) follow(ctx context.Context, e trust.Enrollment) error {
	if e.Remote {
		conn, err := c.Authority.DialRelay(ctx, e)
		if err != nil {
			return err
		}
		return c.consume(ctx, conn, e)
	}
	transport := &http.Transport{TLSClientConfig: c.Authority.ClientTLS(e.Fingerprint)}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	dial, cancel := context.WithTimeout(ctx, 15*time.Second)
	conn, _, err := websocket.Dial(dial, "wss://"+e.Address+"/v1/feed", &websocket.DialOptions{HTTPClient: client})
	cancel()
	if err != nil {
		return err
	}
	return c.consume(ctx, conn, e)
}
func (c *Client) consume(ctx context.Context, conn *websocket.Conn, e trust.Enrollment) error {
	if err := c.Store.DiscardCapture(ctx); err != nil {
		conn.CloseNow()
		return err
	}
	defer c.Store.DiscardCapture(context.WithoutCancel(ctx))
	defer conn.CloseNow()
	conn.SetReadLimit(contracts.BlobChunkSize + 64)
	ctx, stop := context.WithCancel(ctx)
	defer stop()
	ctx = priorityWriter(ctx, conn)
	session := &commandSession{conn: conn, ctx: ctx, pending: map[string]chan contracts.CommandResponse{}}
	c.mu.Lock()
	c.session = session
	c.mu.Unlock()
	defer func() {
		c.mu.Lock()
		if c.session == session {
			c.session = nil
		}
		c.mu.Unlock()
		c.update(func(s *FollowStatus) { s.State = "disconnected" })
	}()
	clock := &clockProbe{}
	head := c.Store.Head()
	if _, err := send(ctx, conn, contracts.FeedFrame{T: "Hello", Features: captureFeatures(), DelegationProtocol: delegationProtocol, ClockProtocol: contracts.FeedClockProtocol, Protocol: contracts.FeedProtocol, ReplicaID: e.ReplicaID, Cursor: &head, SchemaOffer: []int{1}, Credits: contracts.ByteCredit}); err != nil {
		return err
	}
	go func() {
		tick := time.NewTicker(5 * time.Second)
		defer tick.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-tick.C:
				if err := clock.ping(ctx, conn); err != nil {
					stop()
					return
				}
				if _, err := send(ctx, conn, contracts.FeedFrame{T: "Heartbeat", At: time.Now().UTC()}); err != nil {
					stop()
					return
				}
			}
		}
	}()
	blobEvents := make(chan blobEvent, 2)
	blobTrigger := make(chan struct{}, 1)
	if c.Blobs != nil {
		go c.followBlobs(ctx, conn, blobEvents, blobTrigger)
	}
	var welcome contracts.FeedFrame
	var staging *state.SnapshotInstall
	for {
		bounded, cancelRead := context.WithTimeout(ctx, 15*time.Second)
		kind, raw, err := conn.Read(bounded)
		received := time.Now()
		cancelRead()
		if err != nil {
			return err
		}
		if kind == websocket.MessageBinary {
			d, offset, b, err := contracts.DecodeBlobChunk(raw)
			if err != nil {
				return err
			}
			select {
			case blobEvents <- blobEvent{digest: d, offset: offset, bytes: b}:
			case <-ctx.Done():
				return ctx.Err()
			}
			continue
		}
		if kind != websocket.MessageText || len(raw) > contracts.MaxFrame {
			return contracts.ErrInvalid
		}
		var f contracts.FeedFrame
		if err = json.Unmarshal(raw, &f); err != nil {
			return err
		}
		if f.T == "ClockPong" {
			if sample, ok := clock.complete(f, received); ok {
				if c.GatewayClock != nil {
					c.GatewayClock(sample)
				}
				if _, err := send(ctx, conn, contracts.FeedFrame{T: "ClockAck", ID: f.ID, Clock: &sample}); err != nil {
					return err
				}
			}
			continue
		}
		if f.T == "DelegationRequest" {
			if welcome.DelegationProtocol != delegationProtocol {
				return contracts.ErrInvalid
			}
			if err := c.answerDelegation(ctx, conn, f); err != nil {
				return err
			}
			continue
		}
		if f.T == "CommandResponse" {
			if f.Response == nil || f.ID == "" {
				return contracts.ErrInvalid
			}
			session.deliver(f.ID, *f.Response)
			continue
		}
		n := len(raw)
		if f.T == "BlobAvailable" {
			select {
			case blobTrigger <- struct{}{}:
			default:
			}
			continue
		}
		if f.T == "BlobDone" || f.T == "BlobUnavailable" {
			select {
			case blobEvents <- blobEvent{digest: f.Digest, done: true, reason: f.Reason}:
			case <-ctx.Done():
				return ctx.Err()
			}
			continue
		}
		c.update(func(s *FollowStatus) { s.LastHeartbeat = time.Now().UTC() })
		ack := contracts.FeedFrame{T: "Ack", Credits: n}
		switch f.T {
		case "Welcome":
			ceiling := e.Ceiling
			if ceiling == nil {
				ceiling = e.Grants
			}
			if f.Protocol != contracts.FeedProtocol || f.Schema != 1 || f.Epoch == "" || f.SubscriptionRevision == 0 || !trust.Subset(f.Scope, ceiling) {
				return contracts.ErrInvalid
			}

			if f.ClockProtocol == contracts.FeedClockProtocol {
				clock.enable()
				if err := clock.ping(ctx, conn); err != nil {
					return err
				}
			}
			welcome = f
			e.Grants = f.Scope
			if err := c.Store.Restrict(ctx, (trust.Principal{Grants: f.Scope}).Visibility()); err != nil {
				return err
			}
			if err := c.Authority.SaveEnrollment(e); err != nil {
				return err
			}
			c.update(func(s *FollowStatus) {
				s.GatewayHead = f.Head
				s.Schema = f.Schema
				s.Mode = f.Reason
				if f.Reason == "resume" {
					s.Resumes++
					s.State = "connected"
				}
			})
		case "SnapshotBegin":
			c.update(func(s *FollowStatus) { s.State = "snapshot"; s.SnapshotSequence = f.S; s.SnapshotPages = 0 })
			if welcome.Epoch == "" || staging != nil {
				return contracts.ErrInvalid
			}
			staging, err = c.Store.BeginInstall(ctx, f, welcome.Epoch, welcome.SubscriptionRevision)
			if err != nil {
				return err
			}
		case "SnapshotPage":
			if staging == nil {
				return contracts.ErrInvalid
			}
			for _, r := range f.Records {
				if !((trust.Principal{Grants: e.Grants, Kind: "replica"}).Visibility().Allows(r)) {
					return contracts.ErrUnauthorized
				}
			}
			if err = staging.Page(ctx, f); err != nil {
				return err
			}
			c.update(func(s *FollowStatus) { s.SnapshotPages++ })
		case "SnapshotEnd":
			if staging == nil || f.ID != staging.ID {
				return contracts.ErrInvalid
			}
			cursor, err := staging.InstallSnapshot(ctx)
			if err != nil {
				return err
			}
			staging = nil
			ack.Cursor = &cursor
			c.update(func(s *FollowStatus) { s.State = "connected"; s.Snapshots++ })
		case "Batch":
			if staging != nil || f.Batch == nil || f.Batch.Partial && !slices.Contains(welcome.Features, "capture_atomic") {
				return contracts.ErrInvalid
			}
			for _, change := range f.Batch.Changes {
				if !((trust.Principal{Grants: e.Grants, Kind: "replica"}).Visibility().Allows(change.Record)) {
					return contracts.ErrUnauthorized
				}
			}
			if err = commandApplyDelay(ctx); err != nil {
				return err
			}
			cursor, err := c.Store.ApplyBatch(ctx, *f.Batch)
			if err != nil {
				return err
			}
			if f.Batch.Partial {
				if err = capturePartBoundary(ctx); err != nil {
					return err
				}
			} else {
				ack.Cursor = &cursor
			}
		case "Heartbeat":

			select {
			case blobTrigger <- struct{}{}:
			default:
			}
			c.update(func(s *FollowStatus) { s.GatewayHead = f.Head })
			continue
		case "Reset":
			if err = c.Store.DiscardCapture(ctx); err != nil {
				return err
			}
			if staging != nil {
				return contracts.ErrInvalid
			}
		default:
			return contracts.ErrInvalid
		}
		select {
		case blobTrigger <- struct{}{}:
		default:
		}
		if _, err = send(ctx, conn, ack); err != nil {
			return err
		}
	}
}

func (c *Client) Wait(ctx context.Context, target contracts.Cursor) error {
	iterator, err := c.Store.Changes(ctx, trust.AllVisibility(), c.Store.Head())
	if err != nil {
		return err
	}
	for {
		head := c.Store.Head()
		if head.Epoch != target.Epoch {
			return contracts.ErrReset
		}
		if head.Sequence >= target.Sequence {
			return nil
		}
		if _, err = iterator.Next(ctx); err != nil {
			if errors.Is(err, contracts.ErrReset) {
				head := c.Store.Head()
				if head.Epoch != target.Epoch {
					return contracts.ErrReset
				}
				iterator, err = c.Store.Changes(ctx, trust.AllVisibility(), head)
				if err == nil {
					continue
				}
			}
			return err
		}
	}
}

func IsClosed(err error) bool {
	return errors.Is(err, context.Canceled) || websocket.CloseStatus(err) != -1
}
