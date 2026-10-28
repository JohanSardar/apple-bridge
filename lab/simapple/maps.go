package simapple

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"image"
	"image/png"
	"math"
	"strings"
	"time"
)

func simulateMapsQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if !strings.HasPrefix(op, "location.") && !strings.HasPrefix(op, "maps.") {
		return "", nil, false
	}
	var args map[string]json.RawMessage
	if json.Unmarshal(raw, &args) != nil || args == nil {
		return "invalid", nil, true
	}
	if op == "location.current" {
		if len(args) != 0 {
			return "invalid", nil, true
		}
		result, _ := json.Marshal(map[string]any{"lat": 37.7955, "lon": -122.3937, "accuracy": 5, "timestamp": time.Now().UTC().Format(time.RFC3339Nano), "source": "mac"})
		return "", result, true
	}
	allowed := map[string][]string{"location.geocode": {"address"}, "location.reverse_geocode": {"lat", "lon"}, "maps.search": {"query", "near", "radius"}, "maps.nearby": {"category", "near", "radius"}, "maps.directions": {"from", "to", "mode"}, "maps.eta": {"from", "to", "mode"}}
	keys, ok := allowed[op]
	if !ok {
		return "unsupported", nil, true
	}
	for key := range args {
		found := false
		for _, k := range keys {
			found = found || key == k
		}
		if !found {
			return "invalid", nil, true
		}
	}
	if len(args) == 0 {
		return "invalid", nil, true
	}
	validPoint := func(raw json.RawMessage) bool {
		var p struct {
			Lat *float64 `json:"lat"`
			Lon *float64 `json:"lon"`
		}
		return contracts.DecodeInput(raw, &p) == nil && p.Lat != nil && p.Lon != nil && math.Abs(*p.Lat) <= 90 && math.Abs(*p.Lon) <= 180
	}
	if op == "location.geocode" {
		var address string
		if json.Unmarshal(args["address"], &address) != nil || address == "" || len(address) > 1024 {
			return "invalid", nil, true
		}
	}
	if op == "location.reverse_geocode" && !validPoint(raw) {
		return "invalid", nil, true
	}
	if op == "maps.search" || op == "maps.nearby" {
		key := "query"
		if op == "maps.nearby" {
			key = "category"
		}
		var text string
		if json.Unmarshal(args[key], &text) != nil || text == "" || len(text) > 512 || !validPoint(args["near"]) {
			return "invalid", nil, true
		}
		if radius, ok := args["radius"]; ok {
			var r float64
			if json.Unmarshal(radius, &r) != nil || r <= 0 || r > 100000 {
				return "invalid", nil, true
			}
		}
	}
	if op == "maps.directions" || op == "maps.eta" {
		if !validPoint(args["from"]) || !validPoint(args["to"]) {
			return "invalid", nil, true
		}

		var mode string
		json.Unmarshal(args["mode"], &mode)
		if mode == "transit" && op == "maps.directions" {
			return "unsupported", nil, true
		}
		if mode != "driving" && mode != "walking" && mode != "transit" && mode != "cycling" {
			return "invalid", nil, true
		}
		if op == "maps.eta" {
			return "", json.RawMessage(`{"distance":1800,"duration":600000}`), true
		}
		return "", json.RawMessage(`{"routes":[{"distance":1800,"duration":600000,"steps":[{"instructions":"Continue","distance":1800}],"polyline":[{"lat":37.7955,"lon":-122.3937},{"lat":37.8087,"lon":-122.4098}]}]}`), true
	}
	return "", json.RawMessage(`{"places":[{"name":"Public fixture","address":"San Francisco Ferry Building","lat":37.7955,"lon":-122.3937,"category":"cafe","phone":"","url":""}]}`), true
}

func (f *fakeApple) mapsQuery(op string, raw json.RawMessage) (string, json.RawMessage, []contracts.QueryBlob, bool) {
	if op != "maps.snapshot" {
		code, result, handled := simulateMapsQuery(op, raw)
		return code, result, []contracts.QueryBlob{}, handled
	}
	var args struct {
		Center struct {
			Lat *float64 `json:"lat"`
			Lon *float64 `json:"lon"`
		}
		Span          float64
		Width, Height float64
		Markers       []struct {
			Lat *float64 `json:"lat"`
			Lon *float64 `json:"lon"`
		}
	}
	if contracts.DecodeInput(raw, &args) != nil || args.Center.Lat == nil || args.Center.Lon == nil || math.Abs(*args.Center.Lat)+args.Span/2 > 90 || math.Abs(*args.Center.Lon) > 180 || math.Trunc(args.Width) != args.Width || math.Trunc(args.Height) != args.Height || args.Width < 128 || args.Width > 1024 || args.Height < 128 || args.Height > 1024 || args.Span <= 0 || args.Span > 180 || len(args.Markers) > 50 {
		return "invalid", nil, nil, true
	}
	for _, marker := range args.Markers {
		if marker.Lat == nil || marker.Lon == nil || math.Abs(*marker.Lat) > 90 || math.Abs(*marker.Lon) > 180 {
			return "invalid", nil, nil, true
		}
	}
	var buffer bytes.Buffer
	png.Encode(&buffer, image.NewRGBA(image.Rect(0, 0, int(args.Width), int(args.Height))))
	data := buffer.Bytes()
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	f.mu.Lock()
	f.blobs[digest] = data
	f.mu.Unlock()
	result, _ := json.Marshal(map[string]any{"attribution": "© Apple Maps", "image": map[string]any{"digest": digest, "size": len(data)}})
	return "", result, []contracts.QueryBlob{{Digest: digest, Size: int64(len(data)), Handle: digest}}, true
}
