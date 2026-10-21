package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type effect struct {
	Title  string `json:"title"`
	WallNS int64  `json:"wall_ns"`
}

func live(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	decoder := json.NewDecoder(os.Stdin)
	var in struct {
		input
		Prefix string
	}
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	var head contracts.ReadResult
	if err := request(ctx, in.input, "records.query", contracts.ReadQuery{Limit: 1}, &head); err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, "GET", in.URL+"/v1/changes?after="+head.Cursor.EventID(), nil)
	if err != nil {
		return err
	}
	req.Host = in.Host
	req.Header.Set("Authorization", "Bearer "+in.Token)
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return fmt.Errorf("SSE status %d", response.StatusCode)
	}
	var clock contracts.ClockSample
	raw, err := os.ReadFile(in.Home + "/gateway-clock.json")
	if err != nil {
		return err
	}
	if err = json.Unmarshal(raw, &clock); err != nil {
		return err
	}
	offset, uncertainty, err := clock.Estimate()
	if err != nil {
		return err
	}
	var mu sync.Mutex
	seen := map[string]int64{}
	done := make(chan struct{})
	go func() {
		defer close(done)
		scanner := bufio.NewScanner(response.Body)
		scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			at := time.Now().UnixNano()
			var batch contracts.Batch
			if json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &batch) != nil {
				return
			}
			mu.Lock()
			for _, change := range batch.Changes {
				var data struct {
					Title string `json:"title"`
				}
				if json.Unmarshal(change.Record.Data, &data) == nil && strings.HasPrefix(data.Title, in.Prefix) && seen[data.Title] == 0 {
					seen[data.Title] = at
				}
			}
			mu.Unlock()
		}
	}()
	if err = json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true}); err != nil {
		return err
	}
	var effects []effect
	if err = decoder.Decode(&effects); err != nil {
		return err
	}
	if len(effects) == 0 {
		return fmt.Errorf("no effects")
	}
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		mu.Lock()
		values := []float64{}
		for _, effect := range effects {
			if at := seen[effect.Title]; at != 0 {
				latency := time.Duration(at-effect.WallNS) + offset
				if effect.WallNS <= 0 || latency < -uncertainty {
					mu.Unlock()
					return fmt.Errorf("clock agreement failed")
				}
				values = append(values, max(0, float64(latency)/float64(time.Millisecond)))
			}
		}
		mu.Unlock()
		if len(values) == len(effects) {
			return json.NewEncoder(os.Stdout).Encode(map[string]any{"latencies_ms": values, "clock_uncertainty_ms": float64(uncertainty) / float64(time.Millisecond)})
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-done:
			return fmt.Errorf("live disconnected before convergence")
		case <-ticker.C:
		}
	}
}
