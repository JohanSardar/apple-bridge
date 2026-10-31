package contracts

import (
	"encoding/json"
	"math"
)

func init() {
	RegisterSource("location", "Location", true)
	RegisterSource("maps", "Maps", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "maps.open", Version: 1, Kind: CommandOp, Source: "maps", Scope: "maps:write", Capability: "maps.open", Input: Command{}, Output: CommandRecord{}, Description: "Open Maps at a coordinate; verify Maps is frontmost"},
			{Name: "maps.snapshot", Version: 1, Kind: QueryOp, Source: "maps", Scope: "maps:read", Input: Query{}, Output: QueryResult{}, Description: "Map snapshot PNG"},
			{Name: "location.current", Version: 1, Kind: QueryOp, Source: "location", Scope: "location:read", Input: Query{}, Output: QueryResult{}, Description: "Transient location.current result"},
			{Name: "location.geocode", Version: 1, Kind: QueryOp, Source: "location", Scope: "location:read", Input: Query{}, Output: QueryResult{}, Description: "Transient location.geocode result"},
			{Name: "location.reverse_geocode", Version: 1, Kind: QueryOp, Source: "location", Scope: "location:read", Input: Query{}, Output: QueryResult{}, Description: "Transient location.reverse_geocode result"},
			{Name: "maps.search", Version: 1, Kind: QueryOp, Source: "maps", Scope: "maps:read", Input: Query{}, Output: QueryResult{}, Description: "Transient maps.search result"},
			{Name: "maps.nearby", Version: 1, Kind: QueryOp, Source: "maps", Scope: "maps:read", Input: Query{}, Output: QueryResult{}, Description: "Transient maps.nearby result"},
			{Name: "maps.directions", Version: 1, Kind: QueryOp, Source: "maps", Scope: "maps:read", Input: Query{}, Output: QueryResult{}, Description: "Transient maps.directions result"},
			{Name: "maps.eta", Version: 1, Kind: QueryOp, Source: "maps", Scope: "maps:read", Input: Query{}, Output: QueryResult{}, Description: "Estimate travel time and distance between from/to points with numeric lat and lon fields. Returns duration in milliseconds and distance in meters."},
		}
	}, 81, 82, 83, 84, 85, 86, 87, 88, 89)
}

type MapsCoordinate struct {
	Lat *float64 `json:"lat"`
	Lon *float64 `json:"lon"`
}

func validMapsCoordinate(p *MapsCoordinate) bool {
	return p != nil && p.Lat != nil && p.Lon != nil && math.Abs(*p.Lat) <= 90 && math.Abs(*p.Lon) <= 180
}

type MapsOpenArgs struct {
	From *MapsCoordinate `json:"from,omitempty"`
	To   *MapsCoordinate `json:"to,omitempty"`
	Mode string          `json:"mode,omitempty"`
	Lat  *float64        `json:"lat,omitempty"`
	Lon  *float64        `json:"lon,omitempty"`
}

func ValidateMapsOpen(c Command) error {
	var a MapsOpenArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	if a.From != nil || a.To != nil || a.Mode != "" {
		if a.Lat != nil || a.Lon != nil || !validMapsCoordinate(a.From) || !validMapsCoordinate(a.To) {
			return ErrInvalid
		}
		switch a.Mode {
		case "driving", "walking", "transit", "cycling":
		default:
			return ErrInvalid
		}
	} else if !validMapsCoordinate(&MapsCoordinate{Lat: a.Lat, Lon: a.Lon}) {
		return ErrInvalid
	}

	return nil
}
func MapsOpenEvidence(raw json.RawMessage, hash string) bool {
	var value struct {
		Frontmost string `json:"frontmost_bundle"`
		Prepared  struct {
			Hash string `json:"request_hash"`
		}
	}
	return json.Unmarshal(raw, &value) == nil && value.Frontmost == "com.apple.Maps" && hash != "" && value.Prepared.Hash == hash
}

func mapsETAArgsSchema() map[string]any {
	point := map[string]any{
		"type": "object", "additionalProperties": false, "required": []string{"lat", "lon"},
		"properties": map[string]any{
			"lat": map[string]any{"type": "number", "minimum": -90, "maximum": 90},
			"lon": map[string]any{"type": "number", "minimum": -180, "maximum": 180},
		},
	}
	return map[string]any{
		"type": "object", "additionalProperties": false, "required": []string{"from", "to", "mode"},
		"properties": map[string]any{
			"from": point, "to": point,
			"mode": map[string]any{"type": "string", "enum": []string{"driving", "walking", "transit", "cycling"}},
		},
	}
}
