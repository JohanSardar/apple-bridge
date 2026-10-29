package operations

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestDocumentBlobLease(t *testing.T) {
	ctx := context.Background()
	home := t.TempDir()
	store, err := state.Open(home)
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	uploads, err := blobs.Open(home, blobs.Config{})
	if err != nil {
		t.Fatal(err)
	}
	defer uploads.Close()
	data := []byte("synthetic document")
	digest, pin, err := uploads.Stage(ctx, bytes.NewReader(data), blobs.Expect{Size: int64(len(data))})
	if err != nil {
		t.Fatal(err)
	}
	defer uploads.Release(ctx, pin)
	owner := trust.Principal{ID: "owner", Grants: []contracts.Grant{"pdf:read"}}
	if err = store.SaveQueryBlobs(ctx, owner.ID, "qry_fixture", "pdf:read", []contracts.QueryBlob{{Digest: digest, Size: int64(len(data))}}); err != nil {
		t.Fatal(err)
	}
	gateway := LocalGateway{State: store, Uploads: uploads}
	raw, _ := json.Marshal(map[string]any{"blob": map[string]string{"query_id": "qry_fixture", "digest": digest}, "text": "synthetic"})
	resolved, err := gateway.documentInput(ctx, owner, raw)
	if err != nil {
		t.Fatal(err)
	}
	var args map[string]string
	if json.Unmarshal(resolved, &args) != nil || args["data_base64"] != base64.StdEncoding.EncodeToString(data) || args["text"] != "synthetic" || args["blob"] != "" {
		t.Fatal("document bytes or options changed")
	}
	for _, principal := range []trust.Principal{{ID: "other", Grants: owner.Grants}, {ID: owner.ID}} {
		if _, err = gateway.documentInput(ctx, principal, raw); !errors.Is(err, contracts.ErrForbidden) {
			t.Fatalf("lease boundary: %v", err)
		}
	}
}
