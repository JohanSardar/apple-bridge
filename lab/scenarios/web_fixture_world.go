package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type webFixtureMetadata struct {
	GenerateMS int64      `json:"generate_ms"`
	SeedMS     int64      `json:"seed_ms"`
	ListMS     int64      `json:"list_ms"`
	Messages   int        `json:"messages"`
	Avatar     *webAvatar `json:"avatar,omitempty"`
}

func webSnapshotWorld(ctx context.Context, s Scenario) (*world.World, webFixtureMetadata, world.FixtureMetrics, error) {
	inputs, _ := json.Marshal(struct {
		Seed    int
		Huge    bool
		Fixture string
		Clock   string
	}{1, s.Web.Huge, s.Web.Fixture, time.Now().UTC().Truncate(time.Hour).Format(time.RFC3339)})
	w, raw, metrics, err := world.NewFixture(ctx, world.FixtureOptions{Kind: fmt.Sprintf("demo-huge-%t-list-%t", s.Web.Huge, s.Web.Fixture == "list"), Key: string(inputs), BuildTimeout: webFixtureBuildBudget}, func(w *world.World) (json.RawMessage, error) {
		started := time.Now()
		dataset := demo.Generate(demo.Options{Seed: 1, Huge: s.Web.Huge})
		generated := time.Since(started).Milliseconds()
		started = time.Now()
		if err := demo.PopulateChunked(w, dataset, 4096); err != nil {
			return nil, err
		}
		metadata := webFixtureMetadata{Messages: dataset.Counts["messages.message"], GenerateMS: generated, SeedMS: time.Since(started).Milliseconds()}
		if s.Web.Fixture == "list" {
			started = time.Now()
			avatar, err := webListFixture(w, false)
			if err != nil {
				return nil, err
			}
			metadata.Avatar = &avatar
			metadata.ListMS = time.Since(started).Milliseconds()
		}
		return json.Marshal(metadata)
	})
	if err != nil {
		return nil, webFixtureMetadata{}, metrics, err
	}
	var metadata webFixtureMetadata
	if err = json.Unmarshal(raw, &metadata); err != nil {
		w.Close()
		return nil, metadata, metrics, err
	}
	return w, metadata, metrics, nil
}
