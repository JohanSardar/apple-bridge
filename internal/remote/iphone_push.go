package remote

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/relay"
)

func IPhonePush(home string) iphone.Push {
	client := &http.Client{Timeout: 10 * time.Second}
	return func(ctx context.Context, token string, packet iphone.Packet, expires time.Time) error {
		var identity relayIdentity
		if read(home, "relay", &identity) != nil || !strings.HasPrefix(identity.Endpoint, "https://") || identity.Credential.Credential == "" {
			return contracts.ErrUnavailable
		}
		raw, err := json.Marshal(relay.IPhonePush{Hostname: identity.Credential.Hostname, Token: token, Packet: packet, Expires: expires.Unix()})
		if err != nil {
			return err
		}
		request, err := http.NewRequestWithContext(ctx, "POST", strings.TrimRight(identity.Endpoint, "/")+"/iphone/push", bytes.NewReader(raw))
		if err != nil {
			return err
		}
		request.Header.Set("Authorization", "Bearer "+identity.Credential.Credential)
		request.Header.Set("Content-Type", "application/json")
		response, err := client.Do(request)
		if err != nil {
			return err
		}
		defer response.Body.Close()
		if response.StatusCode != 204 {
			return contracts.ErrUnavailable
		}
		return nil
	}
}
