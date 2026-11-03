package wire

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"time"
)

func CommandHash(op string, args json.RawMessage, version uint64, expires time.Time, era string) (string, error) {
	var value any
	decoder := json.NewDecoder(bytes.NewReader(args))
	decoder.UseNumber()
	if err := decoder.Decode(&value); err != nil {
		return "", err
	}
	raw, err := json.Marshal(struct {
		Op      string
		Args    any
		Version uint64
		Expires time.Time
		Era     string
	}{op, value, version, expires.UTC(), era})
	if err != nil {
		return "", err
	}
	hash := sha256.Sum256(raw)
	return hex.EncodeToString(hash[:]), nil
}
