package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
)

type Stage struct {
	Stage        string            `json:"stage"`
	Observations []json.RawMessage `json:"observations"`
}

func load(path string) []Stage {
	b, e := os.ReadFile(path)
	if e != nil {
		panic(e)
	}
	var stages []Stage
	if e = json.Unmarshal(b, &stages); e != nil {
		panic(e)
	}
	return stages
}
func main() {
	expected, actual := load(os.Args[1]), load(os.Args[2])
	if len(expected) != len(actual) {
		panic("stage count mismatch")
	}
	count := 0
	for i, s := range expected {
		a := actual[i]
		if s.Stage != a.Stage || len(s.Observations) != len(a.Observations) {
			panic(fmt.Sprintf("%s observation count: Swift=%d Rust=%d", s.Stage, len(s.Observations), len(a.Observations)))
		}
		for j, raw := range s.Observations {
			var value any
			if e := json.Unmarshal(raw, &value); e != nil {
				panic(e)
			}
			want, e := json.Marshal(value)
			if e != nil {
				panic(e)
			}
			var compact bytes.Buffer
			if e = json.Compact(&compact, a.Observations[j]); e != nil {
				panic(e)
			}
			if !bytes.Equal(want, compact.Bytes()) {
				panic(fmt.Sprintf("%s item %d\nSwift %s\nRust  %s", s.Stage, j, want, compact.Bytes()))
			}
			count++
		}
	}
	fmt.Printf("Swift/Go and Rust/gojson byte parity: %d stages, %d observations\n", len(expected), count)
}
