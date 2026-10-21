package operations

import (
	"context"
	"errors"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type uploadForwarder interface {
	ForwardUpload(context.Context, trust.Principal, contracts.UploadStatus) (contracts.UploadStatus, error)
	UploadControl(context.Context, trust.Principal, contracts.UploadRequest) (contracts.UploadStatus, error)
}

func (s *Service) ForwardUpload(ctx context.Context, p trust.Principal, local contracts.UploadStatus) (contracts.UploadStatus, error) {
	if target, ok := s.Target.(uploadForwarder); ok {
		return target.ForwardUpload(ctx, p, local)
	}
	return local, nil
}
func (s *Service) UploadState(ctx context.Context, p trust.Principal, local contracts.UploadStatus) (contracts.UploadStatus, error) {
	target, ok := s.Target.(uploadForwarder)
	if !ok || local.State != "ready" {
		return local, nil
	}
	remote, err := target.UploadControl(ctx, p, contracts.UploadRequest{Action: "status", ID: local.ID})
	if errors.Is(err, contracts.ErrNotFound) || err == nil && remote.State != "ready" {
		local.State = "uploading"
		return local, nil
	}
	if err != nil {
		return local, err
	}
	return uploadReady(local, remote, local.Digest)
}
func (s *Service) AbandonUpload(ctx context.Context, p trust.Principal, id string) error {
	target, ok := s.Target.(uploadForwarder)
	if !ok {
		return nil
	}
	_, err := target.UploadControl(ctx, p, contracts.UploadRequest{Action: "abandon", ID: id})
	if errors.Is(err, contracts.ErrNotFound) {
		return nil
	}
	return err
}

func (s *Service) UploadForwarding() bool {
	_, ok := s.Target.(uploadForwarder)
	return ok
}
