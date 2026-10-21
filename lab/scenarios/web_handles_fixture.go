package scenarios

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func webHandlesFixture(w *world.World) (map[string]any, error) {
	avatar, err := webAvatarFixture(w)
	if err != nil {
		return nil, err
	}
	err = rmPut(w,
		rmObservation("messages.handle", "route-handle-photo", map[string]any{"address": "photo@example.com", "service": "iMessage"}),
		rmObservation("messages.handle", "route-handle-plain", map[string]any{"address": "plain@example.com", "service": "SMS"}),
	)
	if err != nil {
		return nil, err
	}
	ids := make([]string, 2)
	err = w.Await(10*time.Second, func() bool {
		aliases, err := gatewayAliases(w)
		if err != nil {
			return false
		}
		ids[0] = aliases[aliasKey(contracts.Messages, "messages.handle", "route-handle-photo")]
		ids[1] = aliases[aliasKey(contracts.Messages, "messages.handle", "route-handle-plain")]
		return ids[0] != "" && ids[1] != ""
	})
	return map[string]any{"ids": ids, "avatar": avatar}, err
}
