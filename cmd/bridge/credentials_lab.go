//go:build lab

package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"flag"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func init() {
	credentialProbe = runCredentialProbe
	operationCatalogProbe = runOperationCatalogProbe
}

func runCredentialProbe(ctx context.Context, args []string) error {
	set := flag.NewFlagSet("lab-credential-lifecycle", flag.ContinueOnError)
	home := set.String("home", "", "isolated credential home")
	if err := set.Parse(args); err != nil {
		return err
	}
	if *home == "" || set.NArg() != 0 {
		return contracts.ErrInvalid
	}
	store := trust.NewSecretStore(*home)
	result := struct {
		Backend string `json:"backend"`
		Created bool   `json:"created"`
		Used    bool   `json:"used"`
		Rotated bool   `json:"rotated"`
		Revoked bool   `json:"revoked"`
	}{Backend: store.Backend()}
	first, second := make([]byte, 32), make([]byte, 32)
	if _, err := rand.Read(first); err != nil {
		return err
	}
	if _, err := rand.Read(second); err != nil {
		return err
	}
	defer store.Delete(context.Background(), "lifecycle")
	if err := store.Put(ctx, "lifecycle", first); err != nil {
		return err
	}
	result.Created = true
	actual, err := store.Get(ctx, "lifecycle")
	if err != nil {
		return err
	}
	result.Used = bytes.Equal(first, actual)
	if err := store.Put(ctx, "lifecycle", second); err != nil {
		return err
	}
	actual, err = store.Get(ctx, "lifecycle")
	if err != nil {
		return err
	}
	result.Rotated = bytes.Equal(second, actual) && !bytes.Equal(first, actual)
	if err := store.Delete(ctx, "lifecycle"); err != nil {
		return err
	}
	_, err = store.Get(ctx, "lifecycle")
	result.Revoked = errors.Is(err, contracts.ErrNotFound)
	return json.NewEncoder(os.Stdout).Encode(result)
}

func runOperationCatalogProbe(_ context.Context, args []string) error {
	set := flag.NewFlagSet("lab-operation-catalog", flag.ContinueOnError)
	set.String("home", "", "isolated fixture home")
	if err := set.Parse(args); err != nil {
		return err
	}
	if set.NArg() != 0 {
		return contracts.ErrInvalid
	}
	type operation struct {
		Scope  contracts.Grant `json:"scope"`
		Schema map[string]any  `json:"schema"`
	}
	result := map[string]operation{}
	for _, descriptor := range contracts.Operations {
		result[descriptor.Name] = operation{Scope: descriptor.Scope, Schema: contracts.OperationSchema(descriptor)}
	}
	return json.NewEncoder(os.Stdout).Encode(result)
}
