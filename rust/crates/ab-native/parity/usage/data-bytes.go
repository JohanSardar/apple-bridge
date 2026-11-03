package main

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
)

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 65536), 4*1024*1024)
	for scanner.Scan() {
		var frame struct {
			Observations []struct {
				Data json.RawMessage `json:"data"`
			} `json:"observations"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &frame); err != nil {
			panic(err)
		}
		digests := make([]string, 0, len(frame.Observations))
		for _, row := range frame.Observations {
			if len(row.Data) == 0 {
				digests = append(digests, "absent")
				continue
			}
			var value any
			decoder := json.NewDecoder(bytes.NewReader(row.Data))
			decoder.UseNumber()
			if err := decoder.Decode(&value); err != nil {
				panic(err)
			}
			encoded, err := json.Marshal(value)
			if err != nil {
				panic(err)
			}
			digest := sha256.Sum256(encoded)
			digests = append(digests, hex.EncodeToString(digest[:]))
		}
		encoded, err := json.Marshal(digests)
		if err != nil {
			panic(err)
		}
		fmt.Println(string(encoded))
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
