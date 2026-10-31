package contracts

import "testing"

func TestRemovedAirPlayOperations(t *testing.T) {
	for _, name := range []string{"mac.airplay_routes", "mac.airplay_select"} {
		for _, descriptor := range Operations {
			if descriptor.Name == name {
				t.Fatalf("removed operation %s is still registered", name)
			}
		}
		if MacExtrasQuery(name) {
			t.Fatalf("removed operation %s is still routed", name)
		}
	}
}
