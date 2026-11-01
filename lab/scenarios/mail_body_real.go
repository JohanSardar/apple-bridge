package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runMailBodyReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Metrics: map[string]any{}}
	check := func(name string, ok bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		return ok
	}
	var truth struct {
		Samples []struct {
			Subject  string `json:"subject_digest"`
			Sender   string `json:"sender_digest"`
			Received int64  `json:"received"`
			Body     string `json:"body_digest"`
		} `json:"samples"`
	}
	w, err := mailRealWorld(ctx, level, "mail:read,system:read")
	if !check("mail_native_world", err == nil) {
		fmt.Fprintln(os.Stderr, err)
		e.Conditions = map[string]string{"reason": "The Mail helper could not start. Check the qualification log for the build or permission failure"}
		return e
	}
	defer w.Close()
	{
		tag := "[ab-test] " + e.RunID
		defer func() {
			cleanup, cancel := context.WithTimeout(context.Background(), 60*time.Second)
			defer cancel()
			proof, err := mailRealProof(cleanup, w, "MAIL-CLEANUP", tag)
			e.Passed = check("owned_fixture_cleanup", err == nil && proof["cleanup_passed"]) && e.Passed
			if err != nil {
				if e.Conditions == nil {
					e.Conditions = map[string]string{}
				}
				e.Conditions["cleanup"] = err.Error()
				fmt.Fprintln(os.Stderr, err)
			}
		}()
		proof, problem := mailRealProof(ctx, w, "MAIL-BODY-FIXTURE", tag)
		if problem != nil {
			e.Conditions = map[string]string{"reason": problem.Error()}
			fmt.Fprintln(os.Stderr, problem)
		}
		for _, step := range []string{"mail_responsive", "self_address_available", "sent_to_self", "self_mail_received"} {
			check("fixture_"+step, proof[step])
		}
		if !check("self_sent_fixture_created", problem == nil && proof["sent_to_self"] && proof["self_mail_received"]) {
			return e
		}
		var oracleErr error
		err = w.Await(45*time.Second, func() bool {
			oracleCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
			defer cancel()
			output, err := exec.CommandContext(oracleCtx, "python3", "lab/truth/mail_body.py", tag).Output()
			if err != nil {
				oracleErr = fmt.Errorf("Mail body fixture could not be read. Check Full Disk Access and let Mail finish downloading the test message: %w", err)
				return true
			}
			if err = json.Unmarshal(output, &truth); err != nil {
				oracleErr = fmt.Errorf("Mail body fixture returned an unreadable digest report: %w", err)
				return true
			}
			return len(truth.Samples) > 0
		})
		if oracleErr != nil {
			err = oracleErr
			e.Conditions = map[string]string{"reason": err.Error()}
			fmt.Fprintln(os.Stderr, err)
		}
		if !check("plain_text_fixture_downloaded", err == nil) {
			return e
		}
		e.Metrics["eligible_sample_count"] = len(truth.Samples)
	}
	digest := func(value string) string { return fmt.Sprintf("%x", sha256.Sum256([]byte(value))) }
	err = w.Await(60*time.Second, func() bool {
		var records contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "mail.message", Text: e.RunID, Limit: 10}, &records)
		if err != nil {
			return false
		}
		e.Metrics["query_record_count"] = len(records.Records)
		for _, sample := range truth.Samples {
			for _, r := range records.Records {
				if r.Collection != "mail.message" {
					continue
				}
				var data struct {
					Subject  string    `json:"subject"`
					Sender   string    `json:"from"`
					Received time.Time `json:"received_at"`
				}
				if json.Unmarshal(r.Data, &data) != nil || digest(data.Subject) != sample.Subject || digest(data.Sender) != sample.Sender || data.Received.Unix() != sample.Received {
					continue
				}
				e.Metrics["matched_fixture_count"] = 1
				e.Metrics["expected_body_digest"] = sample.Body
				args, _ := json.Marshal(map[string]string{"message_id": r.ID})
				var result struct {
                    contracts.QueryResult
                    Error struct { Code string `json:"code"` } `json:"error"`
                }
                status, queryErr := world.Request(ctx, w.Gateway, "/v1/op/mail.body", contracts.Query{Args: args, DeadlineMS: 60000}, &result)
                e.Metrics["body_query_http_status"] = status
                for _, code := range []string{"invalid", "unavailable", "unsupported", "not_found", "deadline", "forbidden", "corrupt"} {
                    if result.Error.Code == code { e.Metrics["body_query_error_"+code+"_count"] = 1 }
                }
                err = queryErr
				var body struct {
					State string `json:"state"`
					Text  string `json:"plain_text"`
				}
				if err == nil && json.Unmarshal(result.ResultJSON, &body) == nil {
                    for _, state := range []string{"partial", "offloaded", "downloaded"} {
                        if body.State == state { e.Metrics["body_state_"+state+"_count"] = 1 }
                    }
                    if body.State != "downloaded" { continue }
					e.Metrics["downloaded_body_count"] = 1
					e.Metrics["last_body_digest"] = digest(body.Text)
					if digest(body.Text) != sample.Body {
						continue
					}
					e.Metrics["body_digest"] = sample.Body
					return true
				}
			}
		}
		return false
	})
	e.Passed = check("mail_body_plain_text_digest_equal", err == nil)
	return e
}
