package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func proveUpload(ctx context.Context, a *trust.Authority, client *replication.Client) error {
	data := []byte("synthetic upload")
	digest := trust.Fingerprint(data)
	principal := trust.Principal{ID: "token:fixture", Kind: "token", Grants: []contracts.Grant{"messages:write"}}
	id := "upload_01ARZ3NDEKTSV4RRFFQ69G5FAV"
	control := func(action, chunk string, offset int64) (contracts.UploadStatus, error) {
		request := contracts.UploadRequest{Action: action, ID: id, ChunkID: chunk, Digest: digest, Size: int64(len(data)), Offset: offset}
		raw, err := json.Marshal(request)
		if err != nil {
			return contracts.UploadStatus{}, err
		}
		header, err := a.SignDelegation(principal, id, trust.Fingerprint(raw))
		if err != nil {
			return contracts.UploadStatus{}, err
		}
		response, err := client.UploadControl(ctx, header, raw)
		if err != nil {
			return contracts.UploadStatus{}, err
		}
		if response.Error != nil {
			return contracts.UploadStatus{}, response.Error
		}
		var status contracts.UploadStatus
		err = json.Unmarshal(response.Result, &status)
		return status, err
	}
	if _, err := control("begin", "", 0); err != nil {
		return fmt.Errorf("upload begin: %w", err)
	}
	for index, chunk := range [][]byte{data[:5], data[5:]} {
		offset := int64(0)
		if index == 1 {
			offset = 5
		}
		chunkID := fmt.Sprintf("request_01ARZ3NDEKTSV4RRFFQ69G5FA%d", index)
		if _, err := control("prepare", chunkID, offset); err != nil {
			return fmt.Errorf("upload prepare: %w", err)
		}
		response, err := client.UploadChunk(ctx, chunkID, digest, offset, chunk)
		if err != nil {
			return err
		}
		if response.Error != nil {
			return response.Error
		}
	}
	status, err := control("commit", "", 0)
	if err != nil {
		return err
	}
	if status.Digest != digest || status.State != "ready" || status.Offset != int64(len(data)) {
		return fmt.Errorf("upload status: %+v", status)
	}
	if _, err := control("commit", "", 0); err != nil {
		return err
	}
	return nil
}
