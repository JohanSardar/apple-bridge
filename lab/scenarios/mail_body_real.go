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
	sample := truth.Samples[0]
	digest := func(value string) string { return fmt.Sprintf("%x", sha256.Sum256([]byte(value))) }
	err = w.Await(60*time.Second, func() bool {
		var records contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "mail.message", Text: e.RunID, Limit: 10}, &records)
		if err != nil {
			return false
		}
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
			args, _ := json.Marshal(map[string]string{"message_id": r.ID})
			var result contracts.QueryResult
			_, err = world.Request(ctx, w.Gateway, "/v1/op/mail.body", contracts.Query{Args: args}, &result)
			var body struct {
				State string `json:"state"`
				Text  string `json:"plain_text"`
			}
			return err == nil && json.Unmarshal(result.ResultJSON, &body) == nil && body.State == "downloaded" && digest(body.Text) == sample.Body
		}
		return false
	})
	e.Passed = check("mail_body_plain_text_digest_equal", err == nil)
	if e.Passed {
		e.Metrics["body_digest"] = sample.Body
	}
	return e
}
