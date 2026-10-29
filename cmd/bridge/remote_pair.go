package main

import (
	"bytes"
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func remotePairCode(ctx context.Context, authority *trust.Authority, home, peer, dial string, grants []contracts.Grant, ttl time.Duration, iphone bool) (string, error) {
	if peer != "" || dial != "" {
		return "", contracts.ErrInvalid
	}
	var output bytes.Buffer
	if err := remote.Command(ctx, home, "status", "", "", &output); err != nil {
		return "", err
	}
	var status remote.Status
	if err := json.Unmarshal(output.Bytes(), &status); err != nil {
		return "", err
	}
	if status.State != "on" || status.URL == "" {
		return "", contracts.ErrUnavailable
	}
	if iphone {
		return authority.PairIPhoneCode(status.URL, true, ttl)
	}
	return authority.PairRemote(status.URL, grants, ttl)
}
