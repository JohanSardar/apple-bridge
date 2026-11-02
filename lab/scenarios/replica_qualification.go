package scenarios

import (
	"strings"
)

func replicaQualificationBlocker(s Scenario) string {
	switch s.ID {
	case "CORE-CRED-LIFECYCLE":
		return "credential lifecycle still invokes the Go SecretStore; needs a binary credential lifecycle protocol"
	case "RMT-PRIVATE-CONNECT":
		return "private transport journey still constructs Go frontdoor, trust and remote implementations"
	}
	if !strings.HasPrefix(s.ID, "CORE-") {
		return "replica journey requires native Apple fixtures or a public relay; needs a synthetic binary-driven replica adapter"
	}
	return ""
}
