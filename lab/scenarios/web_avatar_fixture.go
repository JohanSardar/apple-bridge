package scenarios

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

const webFixtureSetupTimeout = 3 * time.Minute

type webAvatar struct {
	Record               string `json:"record"`
	Digest               string `json:"digest"`
	FixtureFlushMS       int64  `json:"fixture_flush_ms"`
	FixtureFlushBudgetMS int64  `json:"fixture_flush_budget_ms"`
}

func webAvatarFixture(w *world.World) (webAvatar, error) {
	out := webAvatar{}
	previousTimeout := w.Control.Timeout
	w.Control.Timeout = webFixtureSetupTimeout
	defer func() { w.Control.Timeout = previousTimeout }()
	if err := w.ControlAction("/blob-fixture", map[string]any{"size": 1, "mime_type": "image/png"}); err != nil {
		return out, err
	}
	image, _ := base64.StdEncoding.DecodeString("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=")
	digest := sha256.Sum256(image)
	out.Digest = hex.EncodeToString(digest[:])
	if err := w.ControlAction("/external-change", map[string]any{"key": "list-contact-photo", "collection": "contacts.contact", "data": map[string]any{"given_name": "Sam", "family_name": "Rivera", "emails": []any{map[string]any{"value": "photo@example.com"}}, "thumbnail_image": map[string]any{"digest": out.Digest, "size": len(image), "state": "present"}}}); err != nil {
		return out, err
	}
	started := time.Now()
	out.FixtureFlushBudgetMS = w.Control.Timeout.Milliseconds()
	flushErr := w.FlushCaptures(w.Control.Timeout)
	out.FixtureFlushMS = time.Since(started).Milliseconds()
	if flushErr != nil {
		return out, flushErr
	}
	err := w.Await(10*time.Second, func() bool {
		aliases, err := gatewayAliases(w)
		if err != nil {
			return false
		}
		out.Record = aliases[aliasKey(contracts.Contacts, "contacts.contact", "list-contact-photo")]
		return out.Record != ""
	})
	if err != nil {
		return out, fmt.Errorf("contact avatar fixture: %w", err)
	}
	return out, nil
}
