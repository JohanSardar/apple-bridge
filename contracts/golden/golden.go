package golden

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func Check(t *testing.T, name string, data []byte) {
	t.Helper()
	_, file, _, _ := runtime.Caller(0)
	path := filepath.Join(filepath.Dir(file), name)
	if os.Getenv("AB_UPDATE_GOLDEN") == "1" {
		if err := os.WriteFile(path, data, 0644); err != nil {
			t.Fatal(err)
		}
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(want, data) {
		t.Fatalf("%s differs; regenerate with AB_UPDATE_GOLDEN=1 after reviewing compatibility", name)
	}
}
