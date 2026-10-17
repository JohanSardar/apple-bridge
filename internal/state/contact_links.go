package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func contactIndex(ctx context.Context, tx *sql.Tx) (map[string]string, error) {
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE collection='contacts.contact' AND deleted=0")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	index := map[string]string{}
	for rows.Next() {
		var raw []byte
		var record contracts.Record
		var data map[string]any
		if err = rows.Scan(&raw); err != nil {
			return nil, err
		}
		if json.Unmarshal(raw, &record) != nil || json.Unmarshal(record.Data, &data) != nil {
			return nil, contracts.ErrCorrupt
		}
		for _, handle := range contracts.ContactHandles(data) {
			if previous, exists := index[handle]; exists {
				if previous != record.ID {
					index[handle] = ""
				}
			} else {
				index[handle] = record.ID
			}
		}
	}
	return index, rows.Err()
}

func linkedHandleData(raw json.RawMessage, index map[string]string) (json.RawMessage, error) {
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil {
		return nil, contracts.ErrCorrupt
	}
	var address string
	if value, ok := fields["address"]; ok && json.Unmarshal(value, &address) != nil {
		return nil, contracts.ErrCorrupt
	}
	delete(fields, "contact_id")
	if id := index[contracts.NormalizeContactHandle(address)]; id != "" {
		fields["contact_id"], _ = json.Marshal(id)
	}
	return json.Marshal(fields)
}

func refreshContactLinks(ctx context.Context, tx *sql.Tx, cursor *contracts.Cursor) error {
	index, err := contactIndex(ctx, tx)
	if err != nil {
		return err
	}
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE collection='messages.handle' AND deleted=0 ORDER BY id")
	if err != nil {
		return err
	}
	changed := []contracts.Record{}
	for rows.Next() {
		var raw []byte
		var record contracts.Record
		if err = rows.Scan(&raw); err != nil {
			break
		}
		if json.Unmarshal(raw, &record) != nil {
			err = contracts.ErrCorrupt
			break
		}
		var linked json.RawMessage
		linked, err = linkedHandleData(record.Data, index)
		if err != nil {
			break
		}
		if string(linked) == string(record.Data) {
			continue
		}
		record.Data = linked
		record.Version++
		record.ObservedAt = time.Now().UTC()
		changed = append(changed, record)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return err
	}
	for _, record := range changed {
		cursor.Sequence++
		if err = writeRecord(ctx, tx, record, cursor.Sequence); err != nil {
			return err
		}
		if err = appendChange(ctx, tx, *cursor, record, "upsert"); err != nil {
			return err
		}
	}
	return nil
}

func projectContactLink(record contracts.Record, visibility trust.Visibility) (contracts.Record, error) {
	if record.Collection != "messages.handle" || visibility[contracts.Contacts] {
		return record, nil
	}
	data, err := linkedHandleData(record.Data, nil)
	record.Data = data
	return record, err
}
