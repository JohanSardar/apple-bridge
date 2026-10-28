package scenarios

import (
	"fmt"
	"reflect"
	"strings"

	"gopkg.in/yaml.v3"
)

type blobOptions struct {
	Rows int    `yaml:"rows,omitempty"`
	Case string `yaml:"case"`
}
type fixtureCleanupOptions struct {
	Cleanup string `yaml:"cleanup"`
}

type robustnessOptions struct {
	Case  string   `yaml:"case"`
	Paths []string `yaml:"paths"`
}
type worldOptions struct {
	GatewayDials bool `yaml:"gateway_dials"`
	Replicas     int  `yaml:"replicas"`
}
type seedOptions struct {
	Count int `yaml:"count"`
}
type changesOptions struct {
	Count     int `yaml:"count"`
	PerSecond int `yaml:"per_second"`
}
type partitionOptions struct {
	Seconds int `yaml:"seconds"`
	Changes int `yaml:"changes"`
}
type latencyOptions struct {
	P95MS   int `yaml:"p95_ms"`
	Samples int `yaml:"samples"`
}
type resumedOptions struct {
	WithoutSnapshot bool `yaml:"without_snapshot"`
}
type loadRejectedOptions struct {
	Path string `yaml:"path"`
}

func validateScenario(s Scenario) error {
	if s.ID == "" || s.Feature == "" || len(s.Levels) == 0 {
		return fmt.Errorf("scenario identity missing")
	}
	for _, level := range s.Levels {
		switch level {
		case "sim", "real", "replica", "agent", "web":
		default:
			return fmt.Errorf("unknown level %q", level)
		}
	}
	if s.Web != nil {
		return validateWeb(s)
	}
	if s.Feature == "messages.read" && strings.HasPrefix(s.ID, "MSG-") {
		_, _, valid := messageScenarioOptions(s)
		if !valid || s.Feature != "messages.read" {
			return fmt.Errorf("invalid Messages read scenario")
		}
		return nil
	}
	for phase, steps := range map[string][]map[string]map[string]any{"given": s.Given, "when": s.When, "then": s.Then, "cleanup": s.Cleanup} {
		for i, step := range steps {
			if len(step) != 1 {
				return fmt.Errorf("%s[%d]: expected exactly one action", phase, i)
			}
			for action, options := range step {
				var target any
				switch phase + "/" + action {
				case "when/relay_service", "when/fixture_snapshot", "when/capture_atomic", "when/helper_ipc", "when/native_media_export_safety", "when/native_media_yield", "when/native_media_posters", "when/reminders_store_perf", "when/reminders_store", "when/media_variants", "when/readmodels", "when/web", "when/usage", "when/notes", "when/messages_commands", "when/calendar_journey", "when/contacts", "when/session_soak", "when/live_backfill", "when/restart_helper", "when/tcc_ready", "when/reminders_external", "when/reminders_crud", "when/reminders_list", "when/reminders_recurring", "when/reminders_permission_loss":
					target = &struct{}{}
				case "cleanup/fixture":
					target = &fixtureCleanupOptions{}
				case "given/world":
					target = &worldOptions{}
				case "given/seed":
					target = &seedOptions{}
				case "when/blobs":
					target = &blobOptions{}
				case "when/robustness":
					target = &robustnessOptions{}
				case "when/disconnect":
					target = &struct{}{}
				case "when/changes":
					target = &changesOptions{}
				case "when/partition":
					target = &partitionOptions{}
				case "then/latency":
					target = &latencyOptions{}
				case "then/resumed":
					target = &resumedOptions{}
				case "then/load_rejected":
					target = &loadRejectedOptions{}
				case "when/gateway_commands", "then/gateway_revoke", "when/command_journey", "when/connect", "when/heal", "when/truth_multiplicity", "then/converge", "then/source_ready", "then/capture_safety", "then/old_sse_cursor", "then/sse_resume", "then/sse_idle", "cleanup/world":
					target = &struct{}{}
				default:
					return fmt.Errorf("%s[%d]: unknown action %q", phase, i, action)
				}
				if options == nil {
					return fmt.Errorf("%s[%d].%s: expected options mapping", phase, i, action)
				}
				schema := reflect.TypeOf(target).Elem()
				for j := 0; j < schema.NumField(); j++ {
					field := schema.Field(j)
					key := field.Tag.Get("yaml")
					if value, exists := options[key]; exists {
						if value == nil || reflect.TypeOf(value).Kind() != field.Type.Kind() {
							return fmt.Errorf("%s[%d].%s.%s: expected %s", phase, i, action, key, field.Type.Kind())
						}
					}
				}
				raw, err := yaml.Marshal(options)
				if err != nil {
					return err
				}
				decoder := yaml.NewDecoder(strings.NewReader(string(raw)))
				decoder.KnownFields(true)
				if err = decoder.Decode(target); err != nil {
					return fmt.Errorf("%s[%d].%s: %w", phase, i, action, fmt.Errorf("%s: %w", strings.TrimSpace(string(raw)), err))
				}
				for key, value := range options {
					if n, ok := value.(int); ok && (n < 0 || (n == 0 && !(phase == "given" && action == "seed"))) {
						return fmt.Errorf("%s[%d].%s.%s: expected positive integer", phase, i, action, key)
					}
				}
				if opt, ok := target.(*loadRejectedOptions); ok && opt.Path == "" {
					return fmt.Errorf("load_rejected.path required")
				}
			}
		}
	}
	return nil
}
