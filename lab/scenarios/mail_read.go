package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os/exec"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type mailTruth struct {
	Counts  map[string]int `json:"counts"`
	Samples []struct {
		Digest string `json:"digest"`
	} `json:"samples"`
}

func runMailRead(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Metrics: map[string]any{}}
	check := func(name string, ok bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		return ok
	}
	cutoff := time.Now().Add(-30 * 24 * time.Hour).Unix()
	output, err := exec.CommandContext(ctx, "python3", "lab/truth/mail.py", strconv.FormatInt(cutoff, 10)).Output()
	if !check("mail_truth_read_only_available", err == nil) {
		return e
	}
	var truth mailTruth
	err = json.Unmarshal(output, &truth)
	if !check("mail_truth_25_random_samples", err == nil && len(truth.Samples) == 25) {
		return e
	}
	w, err := world.NewRealWithSetup(ctx, "real", 0, world.NativeSetup{FilesystemTruth: true}, world.BlobOptions{Scope: "mail:read,system:read"})
	if !check("mail_native_world", err == nil) {
		return e
	}
	defer w.Close()
	endpoint := w.Gateway
	var digests []string
	var counts map[string]int
	var matched int
	err = w.Await(60*time.Second, func() bool {
		records, readErr := w.Records(endpoint)
		if readErr != nil {
			return false
		}
		counts = map[string]int{}
		matched = 0
		actual := map[string]bool{}
		for _, r := range records {
			if r.Collection != "mail.message" {
				continue
			}
			var data struct {
				Subject   string   `json:"subject"`
				From      string   `json:"from"`
				Received  string   `json:"received_at"`
				Mailboxes []string `json:"mailbox_ids"`
			}
			if json.Unmarshal(r.Data, &data) != nil {
				return false
			}
			date, parseErr := time.Parse(time.RFC3339Nano, data.Received)
			if parseErr != nil {
				return false
			}
			if date.Unix() < cutoff {
				continue
			}
			for _, box := range data.Mailboxes {
				counts[strings.TrimPrefix(box, "mailbox/")]++
			}
			var encoded bytes.Buffer
			encoder := json.NewEncoder(&encoded)
			encoder.SetEscapeHTML(false)
			encoder.Encode([]string{data.Subject, data.From, date.UTC().Format(time.RFC3339)})
			raw := bytes.TrimSuffix(encoded.Bytes(), []byte("\n"))
			actual[fmt.Sprintf("%x", sha256.Sum256(raw))] = true
		}
		for key := range counts {
			if _, ok := truth.Counts[key]; !ok {
				return false
			}
		}
		for key, count := range truth.Counts {
			if counts[key] != count {
				return false
			}
		}
		digests = nil
		for _, sample := range truth.Samples {
			_, ok := actual[sample.Digest]
			if !ok {
				return false
			}
			matched++
			digests = append(digests, sample.Digest)
		}
		return true
	})
	e.Passed = check("mail_mailbox_counts_and_sample_digests_equal", err == nil)
	e.Metrics["truth_counts"] = truth.Counts
	e.Metrics["captured_counts"] = counts
	e.Metrics["sample_matches"] = matched
	if e.Passed {
		e.Metrics["counts"] = truth.Counts
		e.Metrics["digests"] = digests
	}
	return e
}
