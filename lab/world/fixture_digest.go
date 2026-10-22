package world

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	_ "modernc.org/sqlite"
	"os"
	"path/filepath"
	"strings"
)

func fixtureRecordDigest(ctx context.Context, home string) (string, int, error) {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(home, "state.db")+"?mode=ro&_pragma=query_only(1)")
	if err != nil {
		return "", 0, err
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	rows, err := db.QueryContext(ctx, "SELECT payload FROM records ORDER BY collection,id")
	if err != nil {
		return "", 0, err
	}
	defer rows.Close()
	hash := sha256.New()
	count := 0
	for rows.Next() {
		var raw []byte
		if err = rows.Scan(&raw); err != nil {
			return "", 0, err
		}
		hash.Write(raw)
		hash.Write([]byte{'\n'})
		count++
	}
	if err = rows.Err(); err != nil {
		return "", 0, err
	}
	return hex.EncodeToString(hash.Sum(nil)), count, nil
}

func fixtureHelperDigest(directory string) (string, error) {
	files := map[string]string{}
	for _, name := range []string{"sim-state.json", "sim-progress.json", "contacts-generation"} {
		digest, err := fixtureFileDigest(filepath.Join(directory, name))
		if os.IsNotExist(err) {
			digest = "missing"
		} else if err != nil {
			return "", err
		}
		files[name] = digest
	}
	raw, _ := json.Marshal(files)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:]), nil
}

func fixturePayloadDigest(root string) (string, int, error) {
	root = filepath.Join(root, "sha256")
	files := map[string]string{}
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if os.IsNotExist(err) {
			return nil
		}
		if err != nil {
			return err
		}
		if !entry.Type().IsRegular() {
			return nil
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		parts := strings.Split(filepath.ToSlash(relative), "/")
		if len(parts) != 2 || len(parts[0]) != 2 || len(parts[1]) != 62 {
			return fmt.Errorf("unexpected fixture blob payload path")
		}
		expected := parts[0] + parts[1]
		if _, err = hex.DecodeString(expected); err != nil {
			return err
		}
		digest, err := fixtureFileDigest(path)
		if err != nil {
			return err
		}
		if digest != expected {
			return fmt.Errorf("fixture gateway blob differs")
		}
		files[relative] = digest
		return nil
	})
	if err != nil {
		return "", 0, err
	}
	raw, _ := json.Marshal(files)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:]), len(files), nil
}
