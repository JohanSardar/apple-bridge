package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type uploadProbe struct {
	authority *trust.Authority
	principal trust.Principal
	store     *state.Store
	blobs     *blobs.Store
	client    *replication.Client
	stop      context.CancelFunc
	done      chan struct{}
}

func newUploadProbe(ctx context.Context, w *world.World) (*uploadProbe, error) {
	gateway, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return nil, err
	}
	var cfg world.Config
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err != nil {
		return nil, err
	}
	if err = json.Unmarshal(raw, &cfg); err != nil {
		return nil, err
	}
	code, err := gateway.Pair(cfg.PeerAddress, []contracts.Grant{"messages:write", "system:read"})
	if err != nil {
		return nil, err
	}
	home := filepath.Join(w.Dir, "upload-probe")
	authority, err := trust.Open(home)
	if err != nil {
		return nil, err
	}
	if err = authority.Join(ctx, code); err != nil {
		return nil, err
	}
	_, session, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	principal, err := authority.Principal(session.DeviceID)
	if err != nil {
		return nil, err
	}
	store, err := state.Open(home)
	if err != nil {
		return nil, err
	}
	blobStore, err := blobs.Open(home, blobs.Config{})
	if err != nil {
		store.Close()
		return nil, err
	}
	probe := &uploadProbe{authority: authority, principal: principal, store: store, blobs: blobStore}
	if err = probe.connect(ctx, w); err != nil {
		probe.close()
		return nil, err
	}
	return probe, nil
}

func (p *uploadProbe) connect(ctx context.Context, w *world.World) error {
	if p.stop != nil {
		p.disconnect()
	}
	bounded, stop := context.WithCancel(ctx)
	p.stop = stop
	p.done = make(chan struct{})
	p.client = &replication.Client{Authority: p.authority, Store: p.store, Blobs: p.blobs}
	client, done := p.client, p.done
	go func() { client.Follow(bounded); close(done) }()
	return w.Await(5*time.Second, func() bool { return client.Status().State == "connected" })
}
func (p *uploadProbe) disconnect() {
	p.stop()
	<-p.done
	p.stop = nil
}
func (p *uploadProbe) close() {
	if p.stop != nil {
		p.disconnect()
	}
	p.store.Close()
}
func (p *uploadProbe) control(ctx context.Context, request contracts.UploadRequest) (contracts.UploadStatus, error) {
	raw, err := json.Marshal(request)
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	header, err := p.authority.SignDelegation(p.principal, request.ID, trust.Fingerprint(raw))
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	response, err := p.client.UploadControl(ctx, header, raw)
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	return probeUploadStatus(response)
}
func probeUploadStatus(response contracts.CommandResponse) (contracts.UploadStatus, error) {
	var status contracts.UploadStatus
	if response.Error != nil {
		return status, response.Error
	}
	err := json.Unmarshal(response.Result, &status)
	return status, err
}
func (p *uploadProbe) chunk(ctx context.Context, id, digest string, offset int64, data []byte) (contracts.UploadStatus, error) {
	chunkID := contracts.NewID("upload-chunk")
	_, err := p.control(ctx, contracts.UploadRequest{Action: "prepare", ID: id, Digest: digest, Offset: offset, ChunkID: chunkID})
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	response, err := p.client.UploadChunk(ctx, chunkID, digest, offset, data)
	if err != nil {
		return contracts.UploadStatus{}, err
	}
	return probeUploadStatus(response)
}
