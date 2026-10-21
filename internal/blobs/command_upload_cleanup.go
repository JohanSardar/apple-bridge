package blobs

import (
	"os"
	"path/filepath"
	"time"
)

func (s *Store) cleanupCommandUploads(now time.Time) error {
	root := filepath.Join(s.root, "command-uploads")
	entries, err := os.ReadDir(root)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		if !info.ModTime().Before(now.Add(-time.Hour)) {
			continue
		}
		if err = os.RemoveAll(filepath.Join(root, entry.Name())); err != nil {
			return err
		}
	}
	return nil
}
