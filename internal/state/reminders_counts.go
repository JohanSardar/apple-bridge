package state

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type reminderCounts struct {
	values           map[string]int
	unsupported      map[string]bool
	lists            map[string]int
	tags             map[string]int
	identityVerified bool
	tagsUnsupported  bool
}

func newReminderCounts() reminderCounts {
	return reminderCounts{values: map[string]int{}, unsupported: map[string]bool{}, lists: map[string]int{}, tags: map[string]int{}}
}

func (counts *reminderCounts) add(fields reminderFields, zone *time.Location, today string) {
	assigned, assignmentKnown := reminderAssignee(fields)
	if fields.Assignment != nil && assignmentKnown {
		counts.identityVerified = true
	}
	if fields.Completed {
		counts.values["completed"]++
		return
	}
	counts.values["all"]++
	counts.lists[fields.ListID]++
	day, dueKnown := reminderDueDay(fields.Due, zone)
	if !dueKnown {
		counts.unsupported["today"], counts.unsupported["scheduled"] = true, true
	}
	if day != "" {
		counts.values["scheduled"]++
		if day <= today {
			counts.values["today"]++
		}
	}
	if fields.Flagged == nil {
		counts.unsupported["flagged"] = true
	} else if *fields.Flagged {
		counts.values["flagged"]++
	}
	if !assignmentKnown {
		counts.unsupported["assigned"] = true
	} else if assigned {
		counts.values["assigned"]++
	}
	if fields.Tags == nil {
		counts.tagsUnsupported = true
	}
	seen := map[string]bool{}
	for _, tag := range fields.Tags {
		if tag != "" && !seen[tag] {
			counts.tags[tag]++
			seen[tag] = true
		}
	}
}

func (counts reminderCounts) smart() []contracts.ReminderSmartTile {
	tiles := []contracts.ReminderSmartTile{}
	for _, id := range []string{"today", "scheduled", "all", "flagged", "completed", "assigned"} {
		tile := contracts.ReminderSmartTile{ID: id}
		if counts.unsupported[id] || id == "assigned" && !counts.identityVerified {
			tile.Status = "unsupported"
		} else {
			count := counts.values[id]
			tile.Count = &count
		}
		tiles = append(tiles, tile)
	}
	return tiles
}
