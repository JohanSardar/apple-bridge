package state

import (
	"bytes"
	"slices"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func copyReminderItems(page contracts.ReminderItemsResult) contracts.ReminderItemsResult {
	copyRecord := func(record contracts.Record) contracts.Record {
		record.Data = bytes.Clone(record.Data)
		record.Coverage.Missing = slices.Clone(record.Coverage.Missing)
		record.Coverage.Operations = slices.Clone(record.Coverage.Operations)
		return record
	}
	var copyItems func([]contracts.ReminderItem) []contracts.ReminderItem
	copyItems = func(items []contracts.ReminderItem) []contracts.ReminderItem {
		items = slices.Clone(items)
		for index := range items {
			items[index].Reminder = copyRecord(items[index].Reminder)
			items[index].Subtasks = copyItems(items[index].Subtasks)
			items[index].Attachments = slices.Clone(items[index].Attachments)
			for attachment := range items[index].Attachments {
				items[index].Attachments[attachment] = copyRecord(items[index].Attachments[attachment])
			}
		}
		return items
	}
	page.Sections = slices.Clone(page.Sections)
	for index := range page.Sections {
		page.Sections[index].Items = copyItems(page.Sections[index].Items)
	}
	return page
}

func reminderItemsBytes(key string, page contracts.ReminderItemsResult) int {
	size := len(key) + len(page.Sections)*256
	var add func([]contracts.ReminderItem)
	add = func(items []contracts.ReminderItem) {
		for _, item := range items {
			size += len(item.Reminder.Data) + 512
			for _, attachment := range item.Attachments {
				size += len(attachment.Data) + 512
			}
			add(item.Subtasks)
		}
	}
	for _, section := range page.Sections {
		add(section.Items)
	}
	return size
}
