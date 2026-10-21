package operations

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func uploadReply(value any, err error) (contracts.CommandResponse, error) {
	response := contracts.CommandResponse{}
	if err != nil {
		response.Error = contracts.ErrUnavailable
		var problem *contracts.Error
		if errors.As(err, &problem) {
			response.Error = problem
		}
		return response, nil
	}
	response.Result, err = json.Marshal(value)
	return response, err
}
func (g *LocalGateway) UploadControl(ctx context.Context, cert *x509.Certificate, session, header string, raw json.RawMessage) (contracts.CommandResponse, error) {
	var request contracts.UploadRequest
	if session == "" || contracts.DecodeInput(raw, &request) != nil {
		return uploadReply(nil, contracts.ErrInvalid)
	}
	principal, err := g.Authority.VerifyDelegation(cert, header, request.ID, trust.Fingerprint(raw))
	if err == nil {
		err = g.Authority.Authorize(principal, "messages:write")
	}
	if principal.IsAgent() && principal.Level == trust.AgentRead {
		err = contracts.ErrUnauthorized
	}
	if err != nil {
		return uploadReply(nil, err)
	}
	if request.Action == "begin" || request.Action == "prepare" || request.Action == "commit" {
		if !contracts.ValidDigest(request.Digest) || request.Action == "begin" && request.Size < 0 {
			return uploadReply(nil, contracts.ErrInvalid)
		}
	}
	if g.Uploads == nil {
		return uploadReply(nil, contracts.ErrUnavailable)
	}
	if request.Action == "prepare" {
		result, err := g.Uploads.CheckUploadChunk(ctx, principal.ID, request)
		if err == nil {
			err = g.prepareUpload(ctx, session, trust.Fingerprint(cert.Raw), principal, request)
		}
		return uploadReply(result, err)
	}
	result, err := g.Uploads.Upload(ctx, principal.ID, request)
	return uploadReply(result, err)
}
func (g *LocalGateway) UploadChunk(ctx context.Context, cert *x509.Certificate, session string, raw []byte) (string, contracts.CommandResponse, error) {
	digest, offset, data, err := contracts.DecodeBlobChunk(raw)
	if err != nil {
		return "", contracts.CommandResponse{}, err
	}
	key := uploadBindingKey(session, digest, offset)
	g.uploadMu.Lock()
	binding, ok := g.uploadPending[key]
	delete(g.uploadPending, key)
	g.uploadMu.Unlock()
	if !ok {
		return "", contracts.CommandResponse{}, contracts.ErrUnauthorized
	}
	parent, err := g.Authority.AuthenticateCertificate(cert)
	if err == nil && (binding.Fingerprint != trust.Fingerprint(cert.Raw) || !trust.Subset([]contracts.Grant{"messages:write"}, parent.Grants)) {
		err = contracts.ErrUnauthorized
	}
	if err == nil && !time.Now().Before(binding.Expires) {
		err = contracts.ErrExpired
	}
	var status contracts.UploadStatus
	if err == nil {
		status, err = g.Uploads.AppendUpload(ctx, binding.Principal.ID, binding.Request.ID, offset, data)
	}
	response, problem := uploadReply(status, err)
	return binding.Request.ChunkID, response, problem
}
