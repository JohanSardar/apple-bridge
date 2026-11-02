package world

import (
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Principal struct {
	ID        string            `json:"id"`
	Kind      string            `json:"kind"`
	TokenHash string            `json:"token_hash"`
	Grants    []contracts.Grant `json:"grants"`
}

type Visibility map[contracts.Source]bool

func (p Principal) Visibility() Visibility {
	result := Visibility{contracts.Source("principal:" + p.ID): true}
	if p.Kind == "replica" {
		result["commands:all"] = true
	}
	for _, grant := range p.Grants {
		if grant == "admin" {
			result["commands:all"] = true
			for _, source := range contracts.Sources {
				result[source] = true
			}
			return result
		}
		source, access, ok := strings.Cut(string(grant), ":")
		if ok && contracts.KnownGrant(grant) && (access == "read" || access == "write") {
			result[contracts.Source(source)] = true
		}
	}
	return result
}

func (v Visibility) Allows(record contracts.Record) bool {
	if !v[record.Source] {
		return false
	}
	if record.Collection != "system.command" || v["commands:all"] {
		return true
	}
	var command struct {
		Principal string `json:"principal"`
	}
	if json.Unmarshal(record.Data, &command) != nil {
		return false
	}
	if v[contracts.Source("principal:"+command.Principal)] {
		return true
	}
	parent, child, ok := strings.Cut(command.Principal, "/")
	return ok && parent != "" && !strings.Contains(child, "/") && v[contracts.Source("principal:"+child)]
}

func effectiveGrants(grants, ceiling []contracts.Grant) []contracts.Grant {
	result := []contracts.Grant{}
	allowed := func(grant contracts.Grant) bool {
		for _, limit := range ceiling {
			if limit == "admin" || limit == grant || strings.TrimSuffix(string(grant), ":read")+":write" == string(limit) {
				return true
			}
		}
		return false
	}
	for _, grant := range grants {
		if allowed(grant) {
			result = append(result, grant)
		} else if strings.HasSuffix(string(grant), ":write") {
			read := contracts.Grant(strings.TrimSuffix(string(grant), ":write") + ":read")
			if allowed(read) {
				result = append(result, read)
			}
		}
	}
	return result
}
