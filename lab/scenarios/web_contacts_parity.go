package scenarios

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webContactsParity(w *world.World, unavailable bool) (any, error) {
	if unavailable {
		err := w.ControlAction("/source-state", map[string]any{"source": contracts.Contacts, "state": "permission_missing"})
		return map[string]bool{"ok": err == nil}, err
	}
	key := "web-con-parity-owned"
	if err := w.ControlAction("/contacts", map[string]any{"action": "create", "key": key, "data": map[string]any{"given_name": "Parity", "family_name": "Bridge Test", "organization": "Bridge Test", "nickname": "[ab-test web-groups]"}}); err != nil {
		return nil, err
	}
	if err := w.FlushCaptures(10 * time.Second); err != nil {
		return nil, err
	}
	if err := w.ControlAction("/external-change", map[string]any{"collection": "contacts.group", "key": "web-con-parity-group", "data": map[string]any{"name": "Bridge Test [ab-test web-groups]", "container_id": "default", "writable": true, "member_ids": []any{}}}); err != nil {
		return nil, err
	}
	if err := w.FlushCaptures(10 * time.Second); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return nil, err
	}
	id := aliases[aliasKey(contracts.Contacts, "contacts.contact", key)]
	if id == "" {
		return nil, contracts.ErrUnavailable
	}
	return map[string]string{"contact": id, "group": aliases[aliasKey(contracts.Contacts, "contacts.group", "web-con-parity-group")]}, nil
}
