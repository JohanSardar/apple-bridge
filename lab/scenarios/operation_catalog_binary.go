package scenarios

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
)

type operationCatalogFixture struct {
	Scope  string         `json:"scope"`
	Schema map[string]any `json:"schema"`
}

func binaryOperationCatalog(w *world.World) (map[string]operationCatalogFixture, error) {
	endpoint := w.Gateway
	endpoint.Home = filepath.Join(w.Dir, "operation-catalog-probe")
	raw, err := w.EndpointCommand(endpoint, "lab-operation-catalog")
	if err != nil {
		return nil, err
	}
	if _, err = os.Stat(endpoint.Home); !os.IsNotExist(err) {
		return nil, fmt.Errorf("operation catalog wrote its fixture home")
	}
	var catalog map[string]operationCatalogFixture
	err = json.Unmarshal(raw, &catalog)
	return catalog, err
}
