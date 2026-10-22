package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type contactVisible struct {
	Operation  string `json:"operation"`
	RecordID   string `json:"record_id"`
	WallNS     int64  `json:"wall_ns"`
	ReadWallNS int64  `json:"read_wall_ns"`
}

func contactsCRUD(ctx context.Context) error {
	var in struct {
		input
		Tag string
	}
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		return err
	}
	var clock contracts.ClockSample
	raw, err := os.ReadFile(filepath.Join(in.Home, "gateway-clock.json"))
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
	watcher, err := watchContacts(ctx, in.input, in.Tag)
	if err != nil {
		return err
	}
	defer watcher.close()
	values := []contactVisible{}
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: world.Endpoint{URL: in.URL, Host: in.Host, Token: in.Token}, Protocol: protocol}
		var target contracts.Record
		for _, operation := range []string{"contacts.create", "contacts.update", "contacts.delete"} {
			args := map[string]any{"given_name": in.Tag + " " + protocol, "family_name": "Testfamily", "organization": "Bridge Test", "job_title": "Testjob"}
			if operation != "contacts.create" {
				args = map[string]any{"id": target.ID}
			}
			if operation == "contacts.update" {
				args["job_title"] = "Updated Testjob"
			}
			var command contracts.CommandRecord
			if err := client.Call(ctx, operation, map[string]any{"command_id": contracts.NewCommandID(), "args": args, "expected_version": target.Version, "wait_ms": 15000}, &command); err != nil {
				return err
			}
			if command.Disposition != "locally_observed" || len(command.ObservedRecordIDs) != 1 {
				return fmt.Errorf("contact command unobserved")
			}
			id := command.ObservedRecordIDs[0]
			bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
			visible := false
			firstRead := int64(0)
			for bounded.Err() == nil {
				var result contracts.ReadResult
				if err := request(bounded, in.input, "records.get", contracts.ReadQuery{Collection: "contacts.contact", ID: id, IncludeDeleted: true}, &result); err != nil {
					cancel()
					return err
				}
				if len(result.Records) == 1 {
					target = result.Records[0]
					var data struct {
						Job   string `json:"job_title"`
						Given string `json:"given_name"`
					}
					if json.Unmarshal(target.Data, &data) != nil {
						cancel()
						return fmt.Errorf("invalid contact record")
					}
					visible = target.Deleted == (operation == "contacts.delete") && data.Given == in.Tag+" "+protocol && (operation != "contacts.update" || data.Job == "Updated Testjob")
					if visible {
						if firstRead == 0 {
							firstRead = time.Now().UnixNano()
						}
						observed := watcher.observed(id, data.Job, target.Deleted)
						if observed != 0 {
							values = append(values, contactVisible{Operation: operation, RecordID: id, WallNS: observed, ReadWallNS: firstRead})
							break
						}
						visible = false
					}
				}
				select {
				case <-bounded.Done():
				case <-time.After(10 * time.Millisecond):
				}
			}
			cancel()
			if !visible {
				return fmt.Errorf("contact read your write failed")
			}
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"values": values, "offset_ms": float64(offset) / float64(time.Millisecond), "uncertainty_ms": float64(uncertainty) / float64(time.Millisecond)})
}
