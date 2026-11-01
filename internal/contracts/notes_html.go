package contracts

import (
	"encoding/json"
	"reflect"
	"regexp"
	"strings"
)

var noteHTMLTokens = regexp.MustCompile(`<[^>]*>|[^<]+`)

func NoteHTMLCompatible(raw json.RawMessage, html string) bool {
	var data struct {
		HTML        string            `json:"html"`
		Content     string            `json:"content_status"`
		Blocks      string            `json:"block_status"`
		RoundTrip   string            `json:"round_trip_status"`
		Attachments []json.RawMessage `json:"attachments"`
	}
	if json.Unmarshal(raw, &data) != nil || data.HTML == "" || data.Content != "complete" || data.Blocks != "complete" || data.RoundTrip != "unproven" || len(data.Attachments) != 0 || len(html) > 1000000 || strings.ContainsRune(html, 0) {
		return false
	}
	for _, marker := range []string{"data-notes-read-only", "<table", "<ul", "<ol", "<pre", "<blockquote"} {
		if strings.Contains(data.HTML, marker) {
			return false
		}
	}
	structure := func(value string) []string {
		tokens := noteHTMLTokens.FindAllString(value, -1)
		if strings.Join(tokens, "") != value {
			return nil
		}
		for i, token := range tokens {
			if !strings.HasPrefix(token, "<") {
				tokens[i] = "text"
			}
		}
		return tokens
	}
	original := structure(data.HTML)
	return original != nil && reflect.DeepEqual(original, structure(html))
}
