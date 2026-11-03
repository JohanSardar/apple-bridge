package state

import (
	"bytes"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"slices"
)

func copyThreadResult(page contracts.ThreadResult) contracts.ThreadResult {
	copyRecord := func(record contracts.Record) contracts.Record {
		record.Data = bytes.Clone(record.Data)
		record.Coverage.Missing = slices.Clone(record.Coverage.Missing)
		record.Coverage.Operations = slices.Clone(record.Coverage.Operations)
		return record
	}
	copyRecords := func(records []contracts.Record) []contracts.Record {
		records = slices.Clone(records)
		for i := range records {
			records[i] = copyRecord(records[i])
		}
		return records
	}
	page.Items = slices.Clone(page.Items)
	for i := range page.Items {
		page.Items[i].Record = copyRecord(page.Items[i].Record)
		page.Items[i].Reactions = copyRecords(page.Items[i].Reactions)
		page.Items[i].Attachments = copyRecords(page.Items[i].Attachments)
	}
	return page
}

func threadResultBytes(key string, page contracts.ThreadResult) int {
	size := len(key)
	for _, item := range page.Items {
		size += len(item.Data) + 512
		for _, record := range item.Reactions {
			size += len(record.Data) + 512
		}
		for _, record := range item.Attachments {
			size += len(record.Data) + 512
		}
	}
	return size
}
