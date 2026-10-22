package scenarios

import (
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func notesProjectionCounts(data map[string]any) (map[string]int, error) {
	var blocks []contracts.NotesBlock
	encoded, err := json.Marshal(data["blocks"])
	if err != nil {
		return nil, err
	}
	if err = json.Unmarshal(encoded, &blocks); err != nil {
		return nil, err
	}
	counts := map[string]int{"titles": 0, "headings": 0, "monostyled": 0, "list_items": 0, "checklists": 0, "checked": 0, "quotes": 0, "highlighted_paragraphs": 0, "links": 0, "attachments": 0, "tables": 0, "table_cells": 0}
	links, attachments, tables := map[string]bool{}, map[string]bool{}, map[string]bool{}
	var walk func([]contracts.NotesBlock)
	walk = func(blocks []contracts.NotesBlock) {
		for _, block := range blocks {
			key := map[string]string{"title": "titles", "heading": "headings", "monostyled": "monostyled", "list_item": "list_items", "checklist_item": "checklists"}[block.Type]
			if key != "" {
				counts[key]++
			}
			for _, row := range block.Rows {
				counts["table_cells"] += len(row)
			}
			if block.Checked != nil && *block.Checked {
				counts["checked"]++
			}
			if block.Blockquote != nil && *block.Blockquote {
				counts["quotes"]++
			}
			highlighted := false
			for _, run := range block.Runs {
				highlighted = highlighted || run.Highlight != nil
				if run.Link != nil {
					links[*run.Link] = true
				}
				if run.AttachmentID != nil {
					attachments[*run.AttachmentID] = true
					if run.AttachmentUTI != nil && *run.AttachmentUTI == "com.apple.notes.table" {
						tables[*run.AttachmentID] = true
					}
				}
			}
			if highlighted {
				counts["highlighted_paragraphs"]++
			}
			walk(block.Children)
		}
	}
	walk(blocks)
	counts["links"], counts["attachments"], counts["tables"] = len(links), len(attachments), len(tables)
	return counts, nil
}
