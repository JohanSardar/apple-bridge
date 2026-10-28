package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"image/png"
	"io"
	"math"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
)

func runMapsQueries(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 0)
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("world_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "location:read,maps:read,system:read")
	add("scopes_granted", err == nil)
	if err != nil {
		return
	}
	client := scripted.Client{Endpoint: endpoint}
	if level == "real" {
		err = w.Await(20*time.Second, func() bool {
			var out contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &out) == nil
		})
		add("helper_ready", err == nil)
		if err != nil {
			return
		}
	}
	if s.ID == "LOC-IDLE" || s.ID == "MAP-IDLE" {
		output, pidErr := exec.CommandContext(ctx, "pgrep", "-f", regexp.QuoteMeta(filepath.Join(w.HelperApp, "Contents", "MacOS", "AppleBridgeHelper"))).Output()
		ids := strings.Fields(string(output))
		add("private_helper_pid", pidErr == nil && len(ids) == 1)
		if pidErr != nil || len(ids) != 1 {
			return
		}
		cpu := func() (float64, bool) {
			output, err := exec.CommandContext(ctx, "ps", "-p", ids[0], "-o", "time=").Output()
			fields := strings.Split(strings.TrimSpace(string(output)), ":")
			if err != nil || len(fields) < 2 || len(fields) > 3 {
				return 0, false
			}
			total := 0.0
			for _, field := range fields {
				v, parseErr := strconv.ParseFloat(field, 64)
				if parseErr != nil || v < 0 {
					return 0, false
				}
				total = total*60 + v
			}
			return total, true
		}
		warmup := time.Now()
		quiet := 0
		previous, previousOK := cpu()
		for quiet < 2 && time.Since(warmup) < 5*time.Minute {
			window := time.Now()
			select {
			case <-ctx.Done():
				add("idle_warmup_completed", false)
				return
			case <-time.After(10 * time.Second):
			}
			next, nextOK := cpu()
			if previousOK && nextOK && next >= previous && (next-previous)/time.Since(window).Seconds()*100 < 2 {
				quiet++
			} else {
				quiet = 0
			}
			previous, previousOK = next, nextOK
		}
		add("helper_reached_idle", quiet == 2)
		before, beforeOK := cpu()
		started := time.Now()
		select {
		case <-ctx.Done():
			add("idle_window_completed", false)
			return
		case <-time.After(5 * time.Minute):
		}
		after, afterOK := cpu()
		seconds := time.Since(started).Seconds()
		percent := (after - before) / seconds * 100
		e.Metrics = map[string]any{"idle_cpu_percent": percent, "idle_seconds": seconds}
		add("five_minute_idle_cpu_under_2_percent", beforeOK && afterOK && after >= before && seconds >= 300 && percent < 2)
		var version contracts.QueryResult
		aliveErr := client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &version)
		add("idle_helper_responsive", aliveErr == nil)
		return
	}
	call := func(op, args string) (json.RawMessage, error) {
		var out contracts.QueryResult
		err := client.Call(ctx, op, contracts.Query{Args: json.RawMessage(args), DeadlineMS: 60000}, &out)
		return out.ResultJSON, err
	}
	if s.ID == "LOC-CURRENT-REAL" {
		raw, fixErr := call("location.current", `{}`)
		var fix struct {
			Accuracy  *float64
			Timestamp string
			Source    string
		}
		decodeErr := json.Unmarshal(raw, &fix)
		sampledAt, parseErr := time.Parse(time.RFC3339Nano, fix.Timestamp)
		add("fresh_mac_fix", fixErr == nil && decodeErr == nil && fix.Accuracy != nil && *fix.Accuracy >= 0 && parseErr == nil && time.Since(sampledAt) < 2*time.Minute && time.Since(sampledAt) > -time.Second && fix.Source == "mac")
		return
	}
	if s.ID == "MAP-SNAPSHOT-REAL" {
		var out contracts.QueryResult
		invalidErr := client.Call(ctx, "maps.snapshot", contracts.Query{Args: json.RawMessage(`{"center":{"lat":37.7955,"lon":-122.3937},"span":0.02,"width":2048,"height":512}`), DeadlineMS: 10000}, &out)
		var invalidSize *contracts.Error
		add("oversized_snapshot_rejected", errors.As(invalidErr, &invalidSize) && invalidSize.Code == "invalid")
		q := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"center":{"lat":37.7955,"lon":-122.3937},"span":0.02,"width":512,"height":512,"markers":[{"lat":37.8005,"lon":-122.3937}]}`), DeadlineMS: 60000}
		err = client.Call(ctx, "maps.snapshot", q, &out)
		add("snapshot_blob", err == nil && len(out.Blobs) == 1)
		if err != nil || len(out.Blobs) != 1 {
			return
		}
		req, _ := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+out.Blobs[0].Digest+"?record="+q.ID, nil)
		req.Host = endpoint.Host
		req.Header.Set("Authorization", "Bearer "+endpoint.Token)
		response, readErr := http.DefaultClient.Do(req)
		add("blob_read", readErr == nil)
		if readErr != nil {
			return
		}
		defer response.Body.Close()
		data, readErr := io.ReadAll(io.LimitReader(response.Body, 8<<20))
		config, decodeErr := png.DecodeConfig(bytes.NewReader(data))
		add("png_dimensions", readErr == nil && decodeErr == nil && config.Width == 512 && config.Height == 512)
		markerPixels := 0
		decoded, decodeImageErr := png.Decode(bytes.NewReader(data))
		if decodeImageErr == nil && config.Width == 512 && config.Height == 512 {
			for x := 248; x < 264; x++ {
				for y := 60; y < 220; y++ {
					r, g, b, a := decoded.At(x, y).RGBA()
					if r > 50000 && g < 30000 && b < 30000 && a > 50000 {
						markerPixels++
					}
				}
			}
		}
		add("northern_marker_appears_north", markerPixels >= 10)
		count := 1
		for count < 17 {
			var repeated contracts.QueryResult
			repeatErr := client.Call(ctx, "maps.snapshot", contracts.Query{Args: q.Args, DeadlineMS: 60000}, &repeated)
			if repeatErr != nil || len(repeated.Blobs) != 1 {
				break
			}
			count++
		}
		e.Metrics = map[string]any{"snapshot_count": count, "png_width": config.Width, "png_height": config.Height, "marker_pixels": markerPixels}
		add("completed_snapshots_release_helper_bytes", count == 17)
		return
	}
	if s.ID == "MAP-SIM" {
		var snapshot contracts.QueryResult
		snapshotErr := client.Call(ctx, "maps.snapshot", contracts.Query{Args: json.RawMessage(`{"center":{"lat":37.7955,"lon":-122.3937},"span":0.02,"width":512,"height":512}`), DeadlineMS: 10000}, &snapshot)
		add("snapshot_schema", snapshotErr == nil && len(snapshot.Blobs) == 1 && snapshot.Blobs[0].Size > 0)
		command := contracts.Command{ID: contracts.NewID("cmd"), Op: "maps.open", Args: json.RawMessage(`{"lat":37.7955,"lon":-122.3937}`), WaitMS: 15000}
		var record contracts.CommandRecord
		writeEndpoint := endpoint
		writeEndpoint.Token, err = w.ScopedToken(endpoint, "maps:write,maps:read")
		writer := scripted.Client{Endpoint: writeEndpoint}
		err = writer.Call(ctx, "maps.open", command, &record)
		add("maps_open_observed", err == nil && record.State == "observed")
		command.ID = contracts.NewID("cmd")
		command.Args = json.RawMessage(`{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"walking"}`)
		err = writer.Call(ctx, "maps.open", command, &record)
		add("maps_route_open_observed", err == nil && record.State == "observed")
		command.ID = contracts.NewID("cmd")
		command.Args = json.RawMessage(`{"from":{"lat":37.7955,"lon":-122.3937},"mode":"walking"}`)
		err = writer.Call(ctx, "maps.open", command, &record)
		var invalid *contracts.Error
		add("incomplete_route_rejected", errors.As(err, &invalid) && invalid.Code == "invalid")
	}
	if s.ID == "LOC-SIM" {
		raw, currentErr := call("location.current", `{}`)
		var fix struct {
			Lat, Lon, Accuracy *float64
			Timestamp, Source  string
		}
		decodeErr := json.Unmarshal(raw, &fix)
		sampledAt, parseErr := time.Parse(time.RFC3339Nano, fix.Timestamp)
		add("authorized_fixture_current_schema", currentErr == nil && decodeErr == nil && fix.Lat != nil && fix.Lon != nil && math.Abs(*fix.Lat) <= 90 && math.Abs(*fix.Lon) <= 180 && fix.Accuracy != nil && *fix.Accuracy >= 0 && fix.Source == "mac")
		add("authorized_fixture_current_is_fresh", parseErr == nil && time.Since(sampledAt) >= 0 && time.Since(sampledAt) < 2*time.Minute)
		_, invalidErr := call("location.current", `{"unexpected":true}`)
		var failure *contracts.Error
		add("current_unknown_fields_rejected", errors.As(invalidErr, &failure) && failure.Code == "invalid")
	}
	if s.ID == "MAP-SIM" {
		for _, mode := range []string{"driving", "walking", "transit", "cycling"} {
			_, routeErr := call("maps.directions", `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"`+mode+`"}`)
			var failure *contracts.Error
			add("transport_mode_"+mode, (mode != "transit" && routeErr == nil) || (mode == "transit" && errors.As(routeErr, &failure) && failure.Code == "unsupported"))
		}
		_, etaErr := call("maps.eta", `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"walking"}`)
		add("eta_completed", etaErr == nil)
		_, nearbyErr := call("maps.nearby", `{"category":"coffee","near":{"lat":37.7955,"lon":-122.3937},"radius":5000}`)
		add("nearby_completed", nearbyErr == nil)
	}
	location := strings.HasPrefix(s.ID, "LOC")
	op := "maps.search"
	args := `{"query":"coffee","near":{"lat":37.7955,"lon":-122.3937},"radius":5000}`
	if location {
		op = "location.geocode"
		args = `{"address":"1 Ferry Building, San Francisco, CA 94111"}`
	}
	if strings.Contains(s.ID, "DIRECTIONS") {
		op = "maps.directions"
		args = `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"walking"}`
	}
	var malformed map[string]any
	json.Unmarshal([]byte(args), &malformed)
	malformed["unexpected"] = true
	invalidArgs, _ := json.Marshal(malformed)
	_, err = call(op, string(invalidArgs))
	var problem *contracts.Error
	add("unknown_fields_rejected", errors.As(err, &problem) && problem.Code == "invalid")
	raw, err := call(op, args)
	if err != nil {
		code := "request_failed"
		var failure *contracts.Error
		if errors.As(err, &failure) && failure != nil {
			code = failure.Code
		}
		fmt.Fprintln(os.Stderr, "maps query failure:", op, code)
	}
	add("query_completed", err == nil)
	if strings.Contains(op, "directions") {
		straightLine := math.Hypot(0.0132*111320, 0.0161*111320*math.Cos(37.8021*math.Pi/180))
		minimumDistance, maximumDistance := straightLine*1.2*0.7, straightLine*3*1.3
		var value struct {
			Routes []struct {
				Distance, Duration float64
				Steps              []any
				Polyline           []any
			}
		}
		json.Unmarshal(raw, &value)
		e.Metrics = map[string]any{"route_count": len(value.Routes)}
		if len(value.Routes) > 0 {
			e.Metrics["distance_m"] = value.Routes[0].Distance
			e.Metrics["duration_ms"] = value.Routes[0].Duration
		}
		add("route_shape", len(value.Routes) > 0 && value.Routes[0].Distance >= minimumDistance && value.Routes[0].Distance <= maximumDistance && value.Routes[0].Duration > 10000 && value.Routes[0].Duration < 36000000 && len(value.Routes[0].Steps) > 0 && len(value.Routes[0].Polyline) > 1)
		raw, drivingErr := call("maps.directions", `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"driving"}`)
		value.Routes = nil
		decodeErr := json.Unmarshal(raw, &value)
		add("driving_route", drivingErr == nil && decodeErr == nil && len(value.Routes) > 0 && value.Routes[0].Distance >= minimumDistance && value.Routes[0].Distance <= maximumDistance && value.Routes[0].Duration > 10000 && value.Routes[0].Duration < 36000000)
		for _, mode := range []string{"walking", "transit", "cycling"} {
			etaRaw, etaErr := call("maps.eta", `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"`+mode+`"}`)
			var estimate struct{ Distance, Duration float64 }
			etaDecode := json.Unmarshal(etaRaw, &estimate)
			add("eta_"+mode+"_distance_and_millisecond_duration", etaErr == nil && etaDecode == nil && estimate.Distance >= minimumDistance && estimate.Distance <= maximumDistance && estimate.Duration > 10000 && estimate.Duration < 36000000)
		}
		cycleRaw, cycleErr := call("maps.directions", `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8087,"lon":-122.4098},"mode":"cycling"}`)
		value.Routes = nil
		cycleDecode := json.Unmarshal(cycleRaw, &value)
		add("cycling_route", cycleErr == nil && cycleDecode == nil && len(value.Routes) > 0 && value.Routes[0].Distance >= minimumDistance && value.Routes[0].Distance <= maximumDistance && value.Routes[0].Duration > 10000 && value.Routes[0].Duration < 36000000 && len(value.Routes[0].Polyline) > 1)

	} else {
		var value struct {
			Places []struct {
				Lat, Lon      float64
				Name, Address string
			}
		}
		decodeErr := json.Unmarshal(raw, &value)
		e.Metrics = map[string]any{"place_count": len(value.Places)}
		add("public_places_present", decodeErr == nil && len(value.Places) > 0)
		if !location {
			valid := len(value.Places) > 0
			for _, place := range value.Places {
				dy := (place.Lat - 37.7955) * 111320
				dx := (place.Lon + 122.3937) * 111320 * math.Cos(37.7955*math.Pi/180)
				valid = valid && place.Name != "" && math.Hypot(dx, dy) <= 5000
			}
			add("places_within_radius", valid)
		}
		if location && len(value.Places) > 0 {
			p := value.Places[0]
			e.Metrics["public_point_error_m"] = math.Hypot((p.Lat-37.7955)*111320, (p.Lon+122.3937)*111320*math.Cos(37.7955*math.Pi/180))
			add("known_public_coordinate", math.Hypot((p.Lat-37.7955)*111320, (p.Lon+122.3937)*111320*math.Cos(37.7955*math.Pi/180)) <= 1000)
			add("formatted_address_present", p.Address != "")
			raw, err = call("location.reverse_geocode", `{"lat":37.7955,"lon":-122.3937}`)
			value.Places = nil
			reverseDecode := json.Unmarshal(raw, &value)
			valid := err == nil && reverseDecode == nil && len(value.Places) > 0
			if valid {
				p := value.Places[0]
				valid = p.Address != "" && math.Hypot((p.Lat-37.7955)*111320, (p.Lon+122.3937)*111320*math.Cos(37.7955*math.Pi/180)) <= 1000
			}
			add("reverse_round_trip", valid)
		}
	}
	return
}
