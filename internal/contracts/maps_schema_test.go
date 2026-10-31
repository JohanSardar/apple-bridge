package contracts

import (
	"reflect"
	"testing"
)

func TestMapsETAArgumentSchema(t *testing.T) {
	for _, d := range Operations {
		if d.Name != "maps.eta" {
			continue
		}
		args := OperationSchema(d)["properties"].(map[string]any)["args"].(map[string]any)
		if args["additionalProperties"] != false || !reflect.DeepEqual(args["required"], []string{"from", "to", "mode"}) {
			t.Fatal("ETA must advertise required route arguments and reject unknown fields")
		}
		fields := args["properties"].(map[string]any)
		for _, name := range []string{"from", "to"} {
			point := fields[name].(map[string]any)
			if point["additionalProperties"] != false || !reflect.DeepEqual(point["required"], []string{"lat", "lon"}) {
				t.Fatal(name, "must require lat/lon, not latitude/longitude")
			}
			coords := point["properties"].(map[string]any)
			for field, limit := range map[string]int{"lat": 90, "lon": 180} {
				value := coords[field].(map[string]any)
				if value["type"] != "number" || value["minimum"] != -limit || value["maximum"] != limit {
					t.Fatal(field, "coordinate bounds missing")
				}
			}
			if len(coords) != 2 {
				t.Fatal("unexpected coordinate fields")
			}
		}
		if !reflect.DeepEqual(fields["mode"].(map[string]any)["enum"], []string{"driving", "walking", "transit", "cycling"}) {
			t.Fatal("ETA modes missing")
		}
		return
	}
	t.Fatal("ETA operation is not registered")
}
