package agent

import (
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func notesFolderMatches(w *world.World, title, body string) bool {
	state, err := w.Truth("observe", map[string]any{"source": "notes", "sqlite": true})
	if err != nil {
		return false
	}
	folders := map[any]bool{}
	entries, _ := state["folders"].([]any)
	for _, value := range entries {
		folder, _ := value.(map[string]any)
		if folder["title"] == "Bridge Test" {
			folders[folder["id"]] = true
		}
	}
	matches := 0
	for _, note := range truthRecords(state) {
		text, _ := note["plain_text"].(string)
		if note["title"] == title && folders[note["folder_id"]] && strings.TrimSpace(text) == strings.TrimSpace(body) && note["locked"] == false {
			matches++
		}
	}
	return matches == 1
}
