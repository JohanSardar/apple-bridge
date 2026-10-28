package contracts

import "slices"

type SourceGrants struct {
	Source Source  `json:"source"`
	Title  string  `json:"title"`
	Grants []Grant `json:"grants"`
	Agent  bool    `json:"agent"`
}

var sourceGrants []SourceGrants
var Sources []Source

func RegisterSource(source Source, title string, agent bool, special ...Grant) {
	if _, ok := GrantsForSource(source); ok {
		panic("duplicate source grants")
	}
	grants := append([]Grant{Grant(string(source) + ":read"), Grant(string(source) + ":write")}, special...)
	sourceGrants = append(sourceGrants, SourceGrants{source, title, grants, agent})
	Sources = append(Sources, source)
}

func GrantsForSource(source Source) (SourceGrants, bool) {
	for _, group := range sourceGrants {
		if group.Source == source {
			group.Grants = slices.Clone(group.Grants)
			return group, true
		}
	}
	return SourceGrants{}, false
}

func GrantGroups(agentOnly bool) []SourceGrants {
	out := []SourceGrants{}
	for _, source := range Sources {
		group, _ := GrantsForSource(source)
		if !agentOnly || group.Agent {
			out = append(out, group)
		}
	}
	return out
}

func KnownGrant(grant Grant) bool {
	for _, group := range sourceGrants {
		if slices.Contains(group.Grants, grant) {
			return true
		}
	}
	return false
}

func AgentScopes() []string {
	out := []string{}
	for _, group := range GrantGroups(true) {
		for _, grant := range group.Grants {
			if grant == Grant(string(group.Source)+":read") || grant == Grant(string(group.Source)+":write") {
				out = append(out, string(grant))
			}
		}
	}
	return out
}
