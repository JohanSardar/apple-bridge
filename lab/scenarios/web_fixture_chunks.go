package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"slices"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func snapshotChunkEquivalence(ctx context.Context) (map[string]string, error) {
	dataset := demo.Generate(demo.Options{Seed: 1})
	messages := 0
	for i, record := range dataset.Records {
		if record.Collection == "messages.message" {
			messages++
		}
		if messages == 256 {
			dataset.Records = dataset.Records[:i+1]
			break
		}
	}
	digests := map[string]string{}
	for _, mode := range []string{"single", "chunked"} {
		digest, err := snapshotSeedDigest(ctx, dataset, mode == "chunked")
		if err != nil {
			return digests, fmt.Errorf("%s: %w", mode, err)
		}
		digests[mode] = digest
	}
	return digests, nil
}

func snapshotSeedDigest(ctx context.Context, dataset demo.Dataset, chunked bool) (string, error) {
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "messages:read,contacts:read,system:read"})
	if err != nil {
		return "", err
	}
	defer w.Close()
	if chunked {
		err = demo.PopulateChunked(w, dataset, 64)
	} else {
		err = demo.Populate(w, dataset)
	}
	if err != nil {
		return "", err
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return "", err
	}
	stable := map[string]string{}
	keys := make([]string, 0, len(dataset.Records))
	for _, record := range dataset.Records {
		key := aliasKey(contracts.CollectionSource(record.Collection), record.Collection, record.NativeKey)
		id := aliases[key]
		if id == "" {
			return "", fmt.Errorf("missing seeded alias %s", key)
		}
		stable[id] = key
		keys = append(keys, key)
	}
	var normalize func(any) any
	normalize = func(value any) any {
		switch v := value.(type) {
		case string:
			if key, ok := stable[v]; ok {
				return key
			}
		case map[string]any:
			for k, item := range v {
				v[k] = normalize(item)
			}
		case []any:
			for i, item := range v {
				v[i] = normalize(item)
			}
		}
		return value
	}
	records, err := w.Records(w.Gateway)
	if err != nil {
		return "", err
	}
	byKey := map[string]json.RawMessage{}
	for _, record := range records {
		key, ok := stable[record.ID]
		if !ok {
			continue
		}
		raw, err := json.Marshal(record)
		if err != nil {
			return "", err
		}
		var value any
		if err = json.Unmarshal(raw, &value); err != nil {
			return "", err
		}
		raw, err = json.Marshal(normalize(value))
		if err != nil {
			return "", err
		}
		byKey[key] = raw
	}
	slices.Sort(keys)
	hash := sha256.New()
	for _, key := range keys {
		raw, ok := byKey[key]
		if !ok {
			return "", fmt.Errorf("missing stored record %s", key)
		}
		hash.Write(raw)
		hash.Write([]byte{'\n'})
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}
