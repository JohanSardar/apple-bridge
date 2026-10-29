package contracts

import (
	"encoding/json"
	"net/url"
	"strings"
)

const Safari Source = "safari"

func init() {
	for collection, prefix := range map[string]string{"safari.tab": "sft", "safari.history": "sfh", "safari.bookmark": "sfb", "safari.reading_list": "sfr", "safari.cloud_tab": "sfc"} {
		Collections[collection] = prefix
	}
	RegisterSource(Safari, "Safari", true)
	registerDescriptors(SafariDescriptors)
	messagesShape["safari.tab"] = map[string]fieldShape{"window": requiredField(shapeNumber), "index": requiredField(shapeNumber), "title": requiredField(shapeString), "url": requiredField(shapeString), "active": requiredField(shapeBool)}
	messagesShape["safari.history"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "last_visit": requiredField(shapeTime), "visit_count": requiredField(shapeNumber)}
	messagesShape["safari.bookmark"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "folder_path": requiredField(shapeString)}
	messagesShape["safari.reading_list"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "added": nullableField(shapeTime), "read": requiredField(shapeBool)}
	messagesShape["safari.cloud_tab"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "device": requiredField(shapeString)}
}

type SafariArgs struct {
	URL string `json:"url,omitempty"`
	Tab string `json:"tab,omitempty"`
}

func ValidateSafariCommand(c Command) error {
	var a SafariArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	switch c.Op {
	case "safari.open_url", "safari.add_reading_list":
		u, e := url.Parse(a.URL)
		if e != nil || len(a.URL) > 2048 || u.Hostname() == "" || u.User != nil || (u.Scheme != "https" && u.Scheme != "http") || strings.ContainsAny(a.URL, "\r\n") || a.Tab != "" {
			return ErrInvalid
		}
	default:
		return ErrInvalid
	}
	return nil
}
func SafariCompletionEvidence(raw json.RawMessage, hash string) bool {
	return MacCompletionEvidence(raw, hash)
}
func SafariDescriptors() []OperationDescriptor {
	var out []OperationDescriptor
	for _, op := range []string{"safari.open_url", "safari.add_reading_list"} {
		out = append(out, OperationDescriptor{Name: op, Version: 1, Kind: CommandOp, Source: Safari, Scope: "safari:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Change Safari with native readback"})
	}
	return append(out, OperationDescriptor{Name: "safari.page_text", Version: 1, Kind: QueryOp, Source: Safari, Scope: "safari:read", Input: Query{}, Output: QueryResult{}, Description: "Fetch public tab text in the bridge; logged-in pages are limited"})
}
