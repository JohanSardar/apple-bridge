package scenarios

import (
	"debug/macho"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
)

var notesCrashUUID = regexp.MustCompile(`(?i)^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$`)

func notesRecordHelperBinary(e *Evidence, app string) {
	file, err := macho.Open(filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper"))
	if err != nil {
		return
	}
	defer file.Close()
	for _, load := range file.Loads {
		data := load.Raw()
		if len(data) >= 24 && file.ByteOrder.Uint32(data[:4]) == 0x1b {
			notesAddHelperBinary(e, fmt.Sprintf("%x-%x-%x-%x-%x", data[8:12], data[12:14], data[14:16], data[16:18], data[18:24]))
			return
		}
	}
}

func notesAddHelperBinary(e *Evidence, id string) {
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	ids, _ := e.Metrics["notes_helper_binary_uuids"].([]string)
	if !slices.Contains(ids, id) {
		e.Metrics["notes_helper_binary_uuids"] = append(ids, id)
	}
}

func failOnNotesHelperCrashes(e Evidence, dir string, before map[string]bool) Evidence {
	ids, _ := e.Metrics["notes_helper_binary_uuids"].([]string)
	if len(ids) == 0 {
		return failOnHelperCrashes(e, dir, before)
	}
	filtered := map[string]bool{}
	for name, present := range before {
		filtered[name] = present
	}
	for _, name := range newHelperCrashes(dir, before) {
		bytes, err := os.ReadFile(filepath.Join(dir, name))
		if err != nil {
			continue
		}
		_, body, ok := strings.Cut(string(bytes), "\n")
		if !ok {
			continue
		}
		var report struct {
			Images []struct {
				Name string `json:"name"`
				UUID string `json:"uuid"`
			} `json:"usedImages"`
		}
		if json.Unmarshal([]byte(body), &report) != nil {
			continue
		}
		for _, image := range report.Images {
			if image.Name == "AppleBridgeHelper" && notesCrashUUID.MatchString(image.UUID) {
				if !slices.ContainsFunc(ids, func(id string) bool { return strings.EqualFold(id, image.UUID) }) {
					filtered[name] = true
				}
				break
			}
		}
	}
	return failOnHelperCrashes(e, dir, filtered)
}
