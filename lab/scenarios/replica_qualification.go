package scenarios

import "strings"

func nativeReplicaSimScenario(s Scenario) bool {
	switch s.ID {
	case "CAL-CRUD", "CAL-WINDOW-SLIDE", "CAL-MOVED-OUTSIDE-WINDOW", "CAL-SPLIT-ATOMIC-ALLDAY", "CAL-SPLIT-ATOMIC-TIMED", "CAL-RECUR-THIS", "CAL-RECUR-FUTURE", "REM-CRUD", "NOTE-CRUD":
		return true
	default:
		return false
	}
}

func nativeReplicaRealScenario(s Scenario) bool {
	switch s.ID {
	case "CAL-CREATE-EXTERNAL", "CAL-CRUD", "CON-CRUD", "CON-EXTERNAL", "MSG-INCOMING", "MSG-SEND-FILES", "MSG-SEND-SELF-TEXT", "NOTE-CRUD", "NOTE-EXTERNAL", "REM-CREATE-EXTERNAL", "REM-CRUD", "MAC-REPLICA", "FILE-REPLICA":
		return true
	default:
		return false
	}
}

func replicaQualificationBlocker(s Scenario) string {
	switch s.ID {
	case "RMT-PRIVATE-CONNECT":
		return "harness: lab/scenarios/remote_private_real.go changes AppleBridgeLive; requires isolated private transport adapter"
	case "MAIL-REPLICA", "CALLS-REPLICA":
		return ""
	case "FILE-REPLICA":
		return "harness: lab/scenarios/files_replica.go requires iCloud filesystem and native Helper; requires isolated filesystem adapter"
	case "MAC-REPLICA":
		return "harness: lab/scenarios/mac.go requires native Helper and AppleTruth; requires isolated mac action adapter"
	}
	if strings.HasPrefix(s.ID, "WEB-RELAY-") || s.ID == "CON-RELAY" {
		return "harness: lab/scenarios/relay_connect.go requires public relay and AppleTruth; requires isolated relay and native fixture adapters"
	}
	if !strings.HasPrefix(s.ID, "CORE-") {
		return "harness: native " + s.Feature + " journey requires AppleTruth/Helper; requires isolated binary-driven fixture adapter"
	}
	return ""
}

func replicaRequirements(s Scenario) string {
	base := "ab-runtime: bridge join, token create, start --role replica; ab-trust: file credentials and pinned mTLS; ab-replication: feed/1 snapshot, journal, reconnect; ab-state: schema-v2 replica apply; ab-frontdoor/ab-operations: REST reads"
	switch {
	case strings.Contains(s.ID, "CMD-"), strings.Contains(s.ID, "CRUD"), strings.Contains(s.ID, "SEND"), s.ID == "MAC-REPLICA":
		base += "; ab-operations: RemoteGateway delegated commands and read-your-write"
	}
	if strings.Contains(s.ID, "BLOB") || strings.Contains(s.ID, "MEDIA") || s.ID == "MSG-SEND-FILES" || s.ID == "FILE-REPLICA" {
		base += "; ab-blobs: authorized resumable transfer, digest validation and cap"
	}
	if strings.Contains(s.ID, "GATEWAY-DIALS") {
		base += "; ab-replication: gateway-dials listener"
	}
	if strings.Contains(s.ID, "SCOPE") || strings.Contains(s.ID, "AGENTS") {
		base += "; ab-trust: scoped visibility, revocation and subscription revisions"
	}
	if strings.Contains(s.ID, "RELAY") {
		base += "; ab-trust/ab-replication: relay/1 authenticated WebSocket transport"
	}
	if s.ID == "CORE-DOCTOR" {
		base += "; ab-runtime: doctor"
	}
	if s.ID == "CORE-CRED-LIFECYCLE" {
		base += "; ab-trust: lab-credential-lifecycle --home (JSON backend, created, used, rotated, revoked)"
	}
	if s.ID == "RMT-PRIVATE-CONNECT" {
		base += "; ab-remote: private transport and connect proxy"
	}
	return base
}
