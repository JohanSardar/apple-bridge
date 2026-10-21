package scenarios

import (
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayCommandChecks(w *world.World, e *Evidence, add func(string, bool)) {
	title := "[ab-test " + e.RunID + "] forwarded reminder"
	input := relayProbeInput(w)
	input["Title"] = title
	visible := func() (bool, error) {
		records, err := w.Inspect()
		if err != nil {
			return false, err
		}
		for _, record := range records {
			var data struct {
				Title string `json:"title"`
			}
			if record.Kind == "upsert" && json.Unmarshal(record.Data, &data) == nil && data.Title == title {
				return true, nil
			}
		}
		return false, nil
	}
	var result struct {
		Create     bool `json:"create_observed"`
		CreateRead bool `json:"create_read_your_write"`
		Delete     bool `json:"delete_observed"`
		DeleteRead bool `json:"delete_read_your_write"`
	}
	err := w.RelayExchange("command", input, func() (any, error) {
		found, err := visible()
		if err != nil {
			return nil, err
		}
		if !found {
			return nil, fmt.Errorf("independent create not visible")
		}
		return map[string]bool{"truth_verified": true}, nil
	}, &result)
	add("remote_forwarded_create_delete_and_read_your_write", err == nil && result.Create && result.CreateRead && result.Delete && result.DeleteRead)
	if err != nil {
		return
	}
	found, err := visible()
	add("independent_forwarded_delete_absence", err == nil && !found)
	effects, err := w.Effects()
	once := err == nil && len(effects) == 2
	for _, count := range effects {
		once = once && count == 1
	}
	add("forwarded_commands_execute_once", once)
}
