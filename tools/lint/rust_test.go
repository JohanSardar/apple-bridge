package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestRustComments(t *testing.T) {
	for _, tc := range []struct {
		source, path string
		count        int
	}{
		{`let s = "https://synthetic.invalid/* ok */";`, "rust/crates/ab-wire/src/lib.rs", 0},
		{`let s = br###"// /* */"###; let c = '/';`, "rust/crates/ab-wire/src/lib.rs", 0},
		{"// forbidden\n", "rust/crates/ab-wire/src/lib.rs", 1},
		{"// SAFETY: inherited descriptor\n", "rust/crates/ab-sys/src/lib.rs", 0},
		{"// SAFETY: inherited descriptor\n", "rust/crates/ab-wire/src/lib.rs", 1},
		{`/* forbidden */`, "rust/crates/ab-sys/src/lib.rs", 1},
		{`#[doc = "forbidden"]`, "rust/crates/ab-wire/src/lib.rs", 1},
	} {
		path := filepath.Join(t.TempDir(), "sample.rs")
		if err := os.WriteFile(path, []byte(tc.source), 0600); err != nil {
			t.Fatal(err)
		}
		if got := len(lintRust(tc.path, path)); got != tc.count {
			t.Fatalf("findings=%d want=%d", got, tc.count)
		}
	}
}

func TestCargoBannedFeatures(t *testing.T) {
	for _, feature := range []string{"arbitrary_precision", "preserve_order"} {
		path := filepath.Join(t.TempDir(), "Cargo.toml")
		source := `[dependencies]` + "\n" + `serde_json = { version = "1", features = ["` + feature + `"] }`
		if err := os.WriteFile(path, []byte(source), 0600); err != nil {
			t.Fatal(err)
		}
		if got := len(lintCargo("rust/Cargo.toml", path)); got != 1 {
			t.Fatalf("findings=%d want=1", got)
		}
	}
}
