package scenarios

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

type cleanupItem struct {
	ID        string
	Tag       string
	Container string
}

type runCleanup struct {
	tag        string
	before     map[string]cleanupItem
	created    map[string]cleanupItem
	settle     time.Duration
	path       string
	containers map[string]string
}

func newRunCleanup(run string, before []cleanupItem) (*runCleanup, error) {
	if run == "" || strings.ContainsAny(run, "[]\r\n") {
		return nil, fmt.Errorf("invalid cleanup run")
	}
	c := &runCleanup{tag: "[ab-test " + run + "]", before: map[string]cleanupItem{}, created: map[string]cleanupItem{}}
	for _, item := range before {
		if item.ID == "" {
			return nil, fmt.Errorf("missing baseline identity")
		}
		if _, exists := c.before[item.ID]; exists {
			return nil, fmt.Errorf("duplicate baseline identity")
		}
		c.before[item.ID] = item
	}
	return c, nil
}

func (c *runCleanup) capture(item cleanupItem) error {
	if item.ID == "" || item.Tag != c.tag || item.Container == "" {
		return fmt.Errorf("unowned cleanup item")
	}
	if _, exists := c.before[item.ID]; exists {
		return fmt.Errorf("baseline cleanup item")
	}
	if previous, exists := c.created[item.ID]; exists && previous != item {
		return fmt.Errorf("changed cleanup identity")
	}
	c.created[item.ID] = item
	return c.persist()
}

func (c *runCleanup) finish(observe func() ([]cleanupItem, error), remove func(cleanupItem) error) (map[string]any, error) {
	proof := map[string]any{"before_count": len(c.before), "created_count": len(c.created), "passed": false}
	current, err := observe()
	if err != nil {
		return proof, err
	}
	ids := make([]string, 0, len(c.created))
	for id := range c.created {
		ids = append(ids, id)
	}
	slices.Sort(ids)
	live := map[string]cleanupItem{}
	for _, item := range current {
		live[item.ID] = item
	}
	for _, id := range ids {
		item, exists := live[id]
		if !exists {
			continue
		}
		if item != c.created[id] {
			return proof, fmt.Errorf("cleanup identity changed")
		}
		if err := remove(item); err != nil {
			return proof, err
		}
	}
	deadline := time.Now().Add(c.settle)
	for {
		after, err := observe()
		if err != nil {
			return proof, err
		}
		proof["after_count"] = len(after)
		matches := len(after) == len(c.before)
		for _, item := range after {
			if original, exists := c.before[item.ID]; !exists || original != item {
				matches = false
			}
		}
		if matches {
			proof["passed"] = true
			return proof, nil
		}
		if !time.Now().Before(deadline) {
			return proof, fmt.Errorf("cleanup baseline mismatch")
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func (c *runCleanup) persist() error {
	if c.path == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(c.path), 0700); err != nil {
		return err
	}
	data, err := json.Marshal(map[string]any{"tag": c.tag, "created": c.created, "containers": c.containers})
	if err != nil {
		return err
	}
	temp := c.path + ".tmp"
	if err := os.WriteFile(temp, data, 0600); err != nil {
		return err
	}
	return os.Rename(temp, c.path)
}
