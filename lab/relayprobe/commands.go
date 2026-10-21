package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func named(ctx context.Context, in input, collection, title string) ([]contracts.Record, error) {
	var records []contracts.Record
	after := ""
	for {
		var page contracts.ReadResult
		if err := request(ctx, in, "records.query", contracts.ReadQuery{Collection: collection, Text: title, After: after, Limit: 200}, &page); err != nil {
			return nil, err
		}
		for _, record := range page.Records {
			var data struct {
				Title string `json:"title"`
			}
			if json.Unmarshal(record.Data, &data) == nil && data.Title == title {
				records = append(records, record)
			}
		}
		if page.Next == "" {
			return records, nil
		}
		after = page.Next
	}
}
func command(ctx context.Context) error {
	decoder := json.NewDecoder(os.Stdin)
	var in struct {
		input
		Title string
	}
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	lists, err := named(ctx, in.input, "reminders.list", "Bridge Test")
	if err != nil || len(lists) != 1 {
		return fmt.Errorf("Bridge Test list ambiguous or unavailable")
	}
	var response struct {
		Disposition contracts.Disposition `json:"disposition"`
	}
	create := map[string]any{"command_id": contracts.NewCommandID(), "args": map[string]any{"list_id": lists[0].ID, "title": in.Title}, "wait_ms": 15000}
	if err = request(ctx, in.input, "reminders.create", create, &response); err != nil {
		return err
	}
	if response.Disposition != "locally_observed" {
		return fmt.Errorf("create unobserved")
	}
	records, err := named(ctx, in.input, "reminders.reminder", in.Title)
	if err != nil || len(records) != 1 {
		return fmt.Errorf("create read-your-write failed")
	}
	if err = json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true}); err != nil {
		return err
	}
	var signal any
	if err = decoder.Decode(&signal); err != nil {
		return err
	}
	remove := map[string]any{"command_id": contracts.NewCommandID(), "args": map[string]any{"id": records[0].ID}, "expected_version": records[0].Version, "wait_ms": 15000}
	if err = request(ctx, in.input, "reminders.delete", remove, &response); err != nil {
		return err
	}
	if response.Disposition != "locally_observed" {
		return fmt.Errorf("delete unobserved")
	}
	records, err = named(ctx, in.input, "reminders.reminder", in.Title)
	if err != nil || len(records) != 0 {
		return fmt.Errorf("delete read-your-write failed")
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{"create_observed": true, "create_read_your_write": true, "delete_observed": true, "delete_read_your_write": true})
}
