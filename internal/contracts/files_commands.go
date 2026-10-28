package contracts

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"path"
	"slices"
	"strings"
)

type FileArgs struct {
	Root    string   `json:"root"`
	Path    string   `json:"path,omitempty"`
	Parent  string   `json:"parent,omitempty"`
	Name    string   `json:"name,omitempty"`
	Content string   `json:"content_base64,omitempty"`
	Digest  string   `json:"digest,omitempty"`
	Tags    []string `json:"tags,omitempty"`
}

func ValidateFileCommand(c Command) error {
	var a FileArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil || !strings.HasPrefix(a.Root, "/") {
		return ErrInvalid
	}
	safe := func(p string) bool {
		if strings.HasPrefix(p, "/") || strings.ContainsRune(p, 0) {
			return false
		}
		for _, s := range strings.Split(p, "/") {
			if s == ".." || s == "." {
				return false
			}
		}
		return true
	}
	if !safe(a.Path) || !safe(a.Parent) {
		return ErrInvalid
	}
	switch c.Op {
	case "files.create", "files.write", "files.move", "files.rename":
		if a.Name == "" || strings.Contains(a.Name, "/") || !safe(a.Name) {
			return ErrInvalid
		}
	case "files.trash", "files.set_tags", "files.download", "files.evict":
	default:
		return ErrUnsupported
	}
	if c.Op != "files.create" && c.Op != "files.write" && a.Path == "" {
		return ErrInvalid
	}
	if c.Op == "files.create" || c.Op == "files.write" {
		data, err := base64.StdEncoding.DecodeString(a.Content)
		digest := sha256.Sum256(data)
		if err != nil || len(data) > 1<<20 || hex.EncodeToString(digest[:]) != a.Digest {
			return ErrInvalid
		}
	}
	if len(a.Tags) > 64 {
		return ErrInvalid
	}
	return nil
}

func FileCompletionEvidence(c Command, raw json.RawMessage, hash string) bool {
	var e struct {
		Observed    bool     `json:"observed"`
		Fingerprint string   `json:"fingerprint"`
		Path        string   `json:"path"`
		Digest      string   `json:"digest"`
		Trashed     bool     `json:"trashed"`
		Downloaded  *bool    `json:"downloaded"`
		Tags        []string `json:"tags"`
		TrashPath   string   `json:"trash_path"`
	}
	var a FileArgs
	if json.Unmarshal(raw, &e) != nil || json.Unmarshal(c.Args, &a) != nil || !e.Observed || hash == "" || e.Fingerprint != hash {
		return false
	}
	if c.Op == "files.trash" {
		return e.Trashed && (strings.Contains(e.TrashPath, "/.Trash/") || strings.Contains(e.TrashPath, "/.Trashes/"))
	}
	expected := path.Join(a.Root, a.Path)
	if c.Op == "files.create" || c.Op == "files.write" || c.Op == "files.move" || c.Op == "files.rename" {
		expected = path.Join(a.Root, a.Parent, a.Name)
	}
	if e.Path != expected {
		return false
	}
	if c.Op == "files.create" || c.Op == "files.write" {
		return e.Digest == a.Digest
	}
	if c.Op == "files.download" || c.Op == "files.evict" {
		return e.Downloaded != nil && *e.Downloaded == (c.Op == "files.download")
	}
	if c.Op == "files.set_tags" {
		slices.Sort(e.Tags)
		slices.Sort(a.Tags)
		return slices.Equal(e.Tags, a.Tags)
	}
	return true
}
