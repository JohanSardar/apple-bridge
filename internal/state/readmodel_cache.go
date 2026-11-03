package state

import (
	"slices"
	"sync"
)

type readModelPageCache[T any] struct {
	mu       sync.Mutex
	revision string
	pages    map[string]T
	order    []string
	bytes    int
}

func (cache *readModelPageCache[T]) get(revision, key string, copyPage func(T) T) (T, bool) {
	cache.mu.Lock()
	defer cache.mu.Unlock()
	var zero T
	if revision != cache.revision {
		return zero, false
	}
	page, ok := cache.pages[key]
	if !ok {
		return page, false
	}
	cache.order = slices.DeleteFunc(cache.order, func(item string) bool { return item == key })
	cache.order = append(cache.order, key)
	return copyPage(page), true
}

func (cache *readModelPageCache[T]) put(revision, key string, page T, copyPage func(T) T, pageBytes func(string, T) int) {
	size := pageBytes(key, page)
	if size > 8<<20 {
		return
	}
	cache.mu.Lock()
	defer cache.mu.Unlock()
	if revision != cache.revision || cache.pages == nil {
		cache.revision = revision
		cache.pages = map[string]T{}
		cache.order = nil
		cache.bytes = 0
	}
	if previous, ok := cache.pages[key]; ok {
		cache.bytes -= pageBytes(key, previous)
		cache.order = slices.DeleteFunc(cache.order, func(item string) bool { return item == key })
	}
	for len(cache.order) > 0 && (len(cache.order) >= 16 || cache.bytes+size > 8<<20) {
		old := cache.order[0]
		cache.order = cache.order[1:]
		cache.bytes -= pageBytes(old, cache.pages[old])
		delete(cache.pages, old)
	}
	cache.pages[key] = copyPage(page)
	cache.order = append(cache.order, key)
	cache.bytes += size
}
