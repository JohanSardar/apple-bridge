package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"io"
)

func (g *LocalGateway) queryBlobChunk(ctx context.Context, p trust.Principal, f forwarded) ([]byte, error) {
	if f.BlobOffset < 0 {
		return nil, contracts.ErrInvalid
	}
	grant, err := g.State.QueryBlobGrant(ctx, p, f.NativeQuery.ID)
	if err != nil {
		return nil, err
	}
	file, err := g.Uploads.OpenAuthorized(ctx, grant, f.BlobDigest)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	if _, err = file.Seek(f.BlobOffset, io.SeekStart); err != nil {
		return nil, err
	}
	return io.ReadAll(io.LimitReader(file, 64<<10))
}
func (s *Service) EnsureQueryBlob(ctx context.Context, p trust.Principal, id, digest string) error {
	remote, ok := s.Target.(*RemoteGateway)
	if !ok {
		return nil
	}
	grant, err := s.State.QueryBlobGrant(ctx, p, id)
	if err != nil {
		return err
	}
	if !grant.AllowsBlob(digest) {
		return contracts.ErrUnauthorized
	}
	file, err := remote.Uploads.OpenAuthorized(ctx, grant, digest)
	if err == nil {
		file.Close()
		return nil
	}
	return remote.Uploads.Transfer(ctx, digest, remoteQueryBlob{remote: remote, principal: p, id: id})
}

type remoteQueryBlob struct {
	remote    *RemoteGateway
	principal trust.Principal
	id        string
}

func (s remoteQueryBlob) Read(ctx context.Context, digest string, offset, length int64) ([]byte, bool, error) {
	var chunk []byte
	err := s.remote.call(ctx, s.principal, forwarded{Method: "query_blob", NativeQuery: &contracts.Query{ID: s.id}, BlobDigest: digest, BlobOffset: offset}, &chunk)
	if len(chunk) > 64<<10 || int64(len(chunk)) > length {
		return nil, false, contracts.ErrCorrupt
	}
	return chunk, len(chunk) < 64<<10, err
}
