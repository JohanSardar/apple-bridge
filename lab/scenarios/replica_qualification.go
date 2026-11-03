package scenarios

import "strings"

func calendarGatewaySimScenario(s Scenario) bool {
	switch s.ID {
	case "CAL-ALLDAY", "CAL-ALLDAY-FUTURE", "CAL-CREATE-EXTERNAL", "CAL-CRUD", "CAL-DST", "CAL-FUTURE-RULE-MISMATCH", "CAL-ID-CHURN", "CAL-LARGE-CAPTURE", "CAL-MOVED-OCCURRENCE", "CAL-MOVED-OUTSIDE-WINDOW", "CAL-PERIODIC-VERIFICATION", "CAL-PERMISSION-LOSS", "CAL-READONLY", "CAL-RECUR-ALL-REJECT", "CAL-RECUR-FUTURE", "CAL-RECUR-THIS", "CAL-SPLIT-ATOMIC-ALLDAY", "CAL-SPLIT-ATOMIC-TIMED", "CAL-SPLIT-FRAME-CAP", "CAL-STAMP-FIELDS", "CAL-WINDOW-SLIDE":
		return true
	default:
		return false
	}
}

func nativeGatewaySimScenario(s Scenario) bool {
	switch s.ID {
	case "HELPER-IPC-RESPONSIVE", "VIS-OCR", "MAC-ACTIONS", "MAC-CAPTURE", "MAC-COMMANDS", "MAC-EXTRAS-SIM", "IWORK-REAL", "FILE-COMMANDS", "FILE-CRUD-REAL", "FILE-OFFLOAD-REAL", "USE-IMPORT", "USE-LOCK-CLIP", "USE-READONLY", "USE-REMOTE-LAG", "AUD-SIM", "LOC-SIM", "MAP-SIM", "PDF-TEXT", "PDF-ANNOTATE":
		return true
	default:
		return false
	}
}

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
		return "harness: private connect fixture calls Go remote implementation; requires isolated binary-driven private transport adapter"
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

func sourceGatewaySimScenario(s Scenario) bool {
	return strings.HasPrefix(s.ID, "CON-") || strings.HasPrefix(s.ID, "NOTE-") || strings.HasPrefix(s.ID, "REM-")
}

func ownerGatewaySimScenario(s Scenario) bool {
	return s.ID == "CONSENT-CONNECTOR" || s.ID == "CORE-REPLICA-CAPTURE-OBSERVERS" || s.ID == "CORE-SOURCE-GRANTS" || s.ID == "WEB-AGENT-LEVELS" || s.ID == "WEB-AGENT-ACTIVITY" || s.ID == "WEB-AGENT-CREDENTIALS" || s.ID == "WEB-AGENT-OAUTH"
}

func phoneGatewaySimScenario(s Scenario) bool {
	switch s.ID {
	case "APP-HEALTH-SIM", "APP-LOC-SIM", "APP-PAIR-SIM", "APP-DEVICE-SIM", "APP-MOTION-SIM", "APP-HEALTH-WRITE-SIM", "APP-ALARM-SIM", "APP-SHELL-SIM", "APP-SCREENTIME-SIM":
		return true
	default:
		return false
	}
}

func relayServiceGatewaySimScenario(s Scenario) bool {
	switch s.ID {
	case "RELAY-ENROLL", "RELAY-LIMITS", "RELAY-PASSTHROUGH":
		return true
	default:
		return false
	}
}
