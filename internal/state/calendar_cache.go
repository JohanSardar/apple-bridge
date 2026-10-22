package state

import (
	"bytes"
	"slices"
	"sync"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type calendarRangeCache struct {
	mu       sync.Mutex
	revision string
	pages    map[string]contracts.CalendarRangeResult
	order    []string
	bytes    int
}

func copyCalendarRange(page contracts.CalendarRangeResult) contracts.CalendarRangeResult {
	copyRecord := func(record contracts.Record) contracts.Record {
		record.Data = bytes.Clone(record.Data)
		record.Coverage.Missing = slices.Clone(record.Coverage.Missing)
		record.Coverage.Operations = slices.Clone(record.Coverage.Operations)
		return record
	}
	page.Events = slices.Clone(page.Events)
	for index := range page.Events {
		page.Events[index].Event = copyRecord(page.Events[index].Event)
	}
	page.Calendars = slices.Clone(page.Calendars)
	for index := range page.Calendars {
		page.Calendars[index] = copyRecord(page.Calendars[index])
	}
	page.CoverageNotes = slices.Clone(page.CoverageNotes)
	return page
}

func calendarRangeBytes(key string, page contracts.CalendarRangeResult) int {
	size := len(key)
	for _, event := range page.Events {
		size += len(event.Event.Data) + 512
	}
	for _, calendar := range page.Calendars {
		size += len(calendar.Data) + 512
	}
	return size
}

func (cache *calendarRangeCache) get(revision, key string) (contracts.CalendarRangeResult, bool) {
	cache.mu.Lock()
	defer cache.mu.Unlock()
	if revision != cache.revision {
		return contracts.CalendarRangeResult{}, false
	}
	page, ok := cache.pages[key]
	if !ok {
		return page, false
	}
	cache.order = slices.DeleteFunc(cache.order, func(item string) bool { return item == key })
	cache.order = append(cache.order, key)
	return copyCalendarRange(page), true
}

func (cache *calendarRangeCache) put(revision, key string, page contracts.CalendarRangeResult) {
	size := calendarRangeBytes(key, page)
	if size > 8<<20 {
		return
	}
	cache.mu.Lock()
	defer cache.mu.Unlock()
	if revision != cache.revision {
		cache.revision = revision
		cache.pages = map[string]contracts.CalendarRangeResult{}
		cache.order = nil
		cache.bytes = 0
	}
	if previous, ok := cache.pages[key]; ok {
		cache.bytes -= calendarRangeBytes(key, previous)
		cache.order = slices.DeleteFunc(cache.order, func(item string) bool { return item == key })
	}
	for len(cache.order) > 0 && (len(cache.order) >= 16 || cache.bytes+size > 8<<20) {
		old := cache.order[0]
		cache.order = cache.order[1:]
		cache.bytes -= calendarRangeBytes(old, cache.pages[old])
		delete(cache.pages, old)
	}
	cache.pages[key] = copyCalendarRange(page)
	cache.order = append(cache.order, key)
	cache.bytes += size
}
