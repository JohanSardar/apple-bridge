package scenarios

import (
	"os"
	"path/filepath"
	"strings"
)

func nativeTestContacts() (map[string]string, error) {
	data, err := os.ReadFile(filepath.Join(os.Getenv("HOME"), ".config", "apple-bridge", "test-contacts.env"))
	result := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		key, value, ok := strings.Cut(strings.TrimPrefix(strings.TrimSpace(line), "export "), "=")
		if ok {
			result[strings.TrimSpace(key)] = strings.Trim(strings.TrimSpace(value), "\"'")
		}
	}
	return result, err
}
