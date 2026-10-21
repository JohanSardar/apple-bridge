package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func stageNativeWebUpload(ctx context.Context, w *world.World, run string, data []byte) (contracts.MessageFile, *webClient, error) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return contracts.MessageFile{}, nil, err
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return contracts.MessageFile{}, nil, err
	}
	c := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	sum := sha256.Sum256(data)
	status, ready := uploadRequest(c, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-Digest": fmt.Sprintf("%x", sum)})
	if status != 200 || ready.State != "ready" {
		return contracts.MessageFile{}, nil, contracts.ErrUnavailable
	}
	return contracts.MessageFile{Name: "ab-" + run + ".txt", UploadID: ready.ID}, c, nil
}
