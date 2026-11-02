package main

import (
	"bytes"
	"context"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func main() {
	store := trust.KeychainStore{Service: "apple-bridge-spike", Path: os.Args[1]}
	value := []byte{0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255}
	if len(os.Args) == 3 && os.Args[2] == "get" {
		got, err := store.Get(context.Background(), "spike")
		if err != nil || !bytes.Equal(got, value) {
			os.Exit(1)
		}
		return
	}
	err := store.Put(context.Background(), "spike", value)
	if err != nil {
		os.Stderr.WriteString("throwaway keychain write failed\n")
		os.Exit(1)
	}
}
