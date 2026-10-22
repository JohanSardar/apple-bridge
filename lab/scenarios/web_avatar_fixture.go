package scenarios

import (
	"context"
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
	Size                 int    `json:"size"`
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
	image, _ := base64.StdEncoding.DecodeString("iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAYAAACqaXHeAAAEEUlEQVR4nO3QSZIbVRRGYe2HAQNWwCr0mmwkZfuaYIQb3OCGKhvWQGPT2GzFu2EJyVXFy/KJCmOXJWWlHeTgm6Wu3n9W629/HkYKNBiwkEEOBZSwgS3soIIaGmihgx4ceAgQxWoJ8L8PcEvGJwo0GLCQQQ4FlLCBLeygghoaaKGDHhx4CBBvLQH2AX4ZRgo0GLCQQQ4FlLCBLeygghoaaKGDHhx4CBDFan1bxicKNBiwkEEOBZSwgS3soIIaGmihgx4ceAgQby8BlgCr9R0ZnyjQYMBCBjkUUMJGfP3Vlx9UyXd7NTTQQgc9OPAQIN65CPDrMFKgwYCFDHIooEyuM/yqWn43aqCFDnpw4CFAFDcS4JDxVyNMF+CujE8UaDBgIYMcCjhm/GUEudNACx304MBDgHh34gCnGD/67AKccvzVCKcN8N1vw0iBBgMWMsihEJMEkLt7LXTQgwMPAaKYJMAU4xlhCXDSAPdkfKJAgwELGeQwaQC530IHPTjwECDemyjAP29eTWaCAC+GkQINBixkkMO0AV7I8Lc66MGBhwBRrNb3ZXyiQIMBCxnkMGkAud9CBz048BAg3l8CLAFW6wcyPlGgwYCFDHKYNIDcb6GDHhx4CBAfXAR4OYwUaDBgIYMcpg3wUoa/1UEPDjwEiGIJsH4o4xMFGgxYyCCHSQPI/RY66MGBhwDx4UQBioki7McvAU4e4Pvfh5ECDQYsZJBDkZx0vNwbtdBBDw48BIhi0gCnilBj/GcXoDwywn78tAEeyfhEgQYDFjLIoYAyOWi8/G7UQAsd9ODAQ4D46AYD7G2uGaKS7yqMnzjAH8NIgQYDFjLIoYASNrCFHVRQQwMtdNCDAw8BolitH8v4RIEGAxYyyKGAEjawhR1UUEMDLXTQgwMPAeLjJcASYLV+IuMTBRoMWMgghwJK2MAWdlBBDQ200EEPDjwEiE8uAvw5jBRoMGAhgxwKKGEDW9hBBTU00EIHPTjwECCKJcD6qYxPFGgwYCGDHAooYQNb2EEFNTTQQgc9OPAQID69oQBf2G8O9tkGOGb0f5kmwA9/DSMFGgxYyCBPphh+VSv/M+qgBwceAkRxsgA3MfxdIT6JAHOMHx0f4EzGJwo0GLCQnc07/jKCvGOvBwceAsSzIwPMPfxqhAMDvBpGCjQYsMncg9+ll3eNHHgIEMVqfS7jEwUaDNjzT3P8ZQR5354DDwHi+QEB5h543QiTBJh72Me4foBnMj5RoMEkc4/6qADy3pGHAPHZRYDXw0iBBiPmHnRYhNcXPASI4loB5h5ybIT3B3gu4xMFGuYecVQAeb+HAPH5EuDDAeYecArvD/Dj38NIgU7mfvxJAsiOUYAolgBLgJ9kfKJAJ3M//iQBZMcoQBT/AmMniyFPeW5SAAAAAElFTkSuQmCC")
	digest := sha256.Sum256(image)
	out.Digest = hex.EncodeToString(digest[:])
	out.Size = len(image)
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
		var contacts contracts.ReadResult
		_, err := world.Request(context.Background(), w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "contacts.contact", IncludeAliases: true, Limit: 1000}, &contacts)
		if err != nil {
			return false
		}
		for _, alias := range contacts.Aliases {
			if alias.NativeKey == "list-contact-photo" {
				out.Record = alias.RecordID
			}
		}
		return out.Record != ""
	})
	if err != nil {
		return out, fmt.Errorf("contact avatar fixture: %w", err)
	}
	return out, nil
}
