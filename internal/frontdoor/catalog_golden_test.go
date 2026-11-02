package frontdoor

import (
	"encoding/json"
	"testing"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestCatalogGolden(t *testing.T) {
	descriptors := []any{}
	for _, d := range contracts.Operations {
		descriptors = append(descriptors, map[string]any{"name": d.Name, "version": d.Version, "kind": d.Kind, "read_model": d.ReadModel, "source": d.Source, "scope": d.Scope, "capability": d.Capability, "description": d.Description, "input": contracts.OperationSchema(d), "output": contracts.JSONSchema(d.Output), "examples": d.Examples})
	}
	for name, value := range map[string]any{"catalog.json": descriptors, "openapi.json": openAPI(contracts.Operations), "collections.json": contracts.Collections} {
		b, err := json.MarshalIndent(value, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		golden.Check(t, name, append(b, '\n'))
	}
}
