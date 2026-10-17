package agent

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"gopkg.in/yaml.v3"
)

type Task struct {
	Kind     string   `yaml:"kind"`
	ID       string   `yaml:"id"`
	Source   string   `yaml:"source"`
	Requires []string `yaml:"requires"`
	Scope    string   `yaml:"scope"`
	Prompt   string   `yaml:"prompt"`
}
type Attempt struct {
	Safety          bool   `json:"safety_check,omitempty"`
	Sends           int    `json:"self_sends,omitempty"`
	CleanupDeferred bool   `json:"cleanup_deferred,omitempty"`
	Number          int    `json:"attempt"`
	Passed          bool   `json:"passed"`
	Truth           bool   `json:"truth_check"`
	Replica         bool   `json:"replica_check"`
	MCPOnly         bool   `json:"mcp_only"`
	Cleanup         bool   `json:"cleanup"`
	Duration        int64  `json:"duration_ms"`
	Failure         string `json:"failure,omitempty"`
}
type TaskResult struct {
	ID       string    `json:"task_id"`
	Passed   bool      `json:"passed"`
	Attempts []Attempt `json:"attempts"`
}
type Evidence struct {
	RunID    string       `json:"run_id"`
	Scenario string       `json:"scenario"`
	Level    string       `json:"level"`
	GitSHA   string       `json:"git_sha"`
	At       time.Time    `json:"at"`
	Passed   bool         `json:"passed"`
	Tasks    []TaskResult `json:"tasks"`
}
type AgentRunner struct {
	Driver  []byte
	Private string
}
type outcome struct {
	Exit      int    `json:"exit_code"`
	Calls     int    `json:"mcp_calls"`
	Forbidden int    `json:"forbidden_calls"`
	Answer    string `json:"answer"`
}

func Suite(ctx context.Context, paths []string, asJSON bool) error {
	selected := false
	for _, path := range paths {
		err := filepath.WalkDir(path, func(path string, entry os.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() || filepath.Ext(path) != ".yaml" {
				return nil
			}
			raw, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			var identity struct {
				ID string `yaml:"id"`
			}
			if err := yaml.Unmarshal(raw, &identity); err != nil {
				return err
			}
			selected = selected || identity.ID == "AGENT-SUITE"
			return nil
		})
		if err != nil {
			return err
		}
	}
	if !selected {
		return fmt.Errorf("no AGENT-SUITE scenario selected")
	}
	tasks, err := loadTasks()
	if err != nil {
		return err
	}
	if err = world.Build(ctx, "replica"); err != nil {
		return err
	}
	sha, err := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	if err != nil {
		return err
	}
	evidence := Evidence{RunID: contracts.NewID("agent"), Scenario: "AGENT-SUITE", Level: "agent", GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Passed: true}
	driver, err := os.ReadFile("lab/agent/codex.py")
	if err != nil {
		return err
	}
	runner := AgentRunner{Driver: driver, Private: filepath.Join("lab/evidence/private", evidence.RunID)}
	if err = os.MkdirAll(runner.Private, 0700); err != nil {
		return err
	}
	for _, task := range tasks {
		result := TaskResult{ID: task.ID}
		successes := 0
		for number := 1; number <= 3; number++ {
			fmt.Fprintln(os.Stderr, "running", task.ID, "attempt", number)
			attempt := runner.run(ctx, task, number)
			fmt.Fprintf(os.Stderr, "%s attempt=%d passed=%t truth=%t replica=%t mcp_only=%t cleanup=%t failure=%s\n", task.ID, number, attempt.Passed, attempt.Truth, attempt.Replica, attempt.MCPOnly, attempt.Cleanup, attempt.Failure)
			result.Attempts = append(result.Attempts, attempt)
			if attempt.Passed {
				successes++
			}
			if ctx.Err() != nil || successes >= 2 || len(result.Attempts)-successes >= 2 {
				break
			}
		}
		result.Passed = successes >= 2
		evidence.Passed = evidence.Passed && result.Passed
		evidence.Tasks = append(evidence.Tasks, result)
	}
	raw, err := json.MarshalIndent(evidence, "", "  ")
	if err != nil {
		return err
	}
	if err = os.MkdirAll("lab/evidence/agent", 0700); err != nil {
		return err
	}
	if err = os.WriteFile("lab/evidence/agent/AGENT-SUITE.json", append(raw, '\n'), 0600); err != nil {
		return err
	}
	if asJSON {
		json.NewEncoder(os.Stdout).Encode(evidence)
	} else {
		fmt.Printf("AGENT-SUITE agent passed=%t tasks=%d\n", evidence.Passed, len(tasks))
	}
	if !evidence.Passed {
		return fmt.Errorf("agent suite failed")
	}
	return nil
}

func loadTasks() ([]Task, error) {
	raw, err := os.ReadFile("features.yaml")
	if err != nil {
		return nil, err
	}
	var graph struct {
		Features []struct {
			ID     string `yaml:"id"`
			Status string `yaml:"status"`
		} `yaml:"features"`
	}
	if err = yaml.Unmarshal(raw, &graph); err != nil {
		return nil, err
	}
	done := map[string]bool{}
	for _, feature := range graph.Features {
		done[feature.ID] = feature.Status == "done"
	}
	files, err := filepath.Glob("lab/agent/tasks/*.yaml")
	if err != nil {
		return nil, err
	}
	tasks := []Task{}
	seen := map[string]bool{}
	for _, file := range files {
		raw, err := os.ReadFile(file)
		if err != nil {
			return nil, err
		}
		var task Task
		decoder := yaml.NewDecoder(bytes.NewReader(raw))
		decoder.KnownFields(true)
		if err = decoder.Decode(&task); err != nil {
			return nil, err
		}
		if task.ID == "" || filepath.Base(task.ID) != task.ID || seen[task.ID] || task.Prompt == "" || len(task.Requires) == 0 {
			return nil, fmt.Errorf("invalid agent task %s", file)
		}
		seen[task.ID] = true
		if !slices.Contains([]string{"reminders", "contacts", "calendar", "messages"}, task.Source) || task.Scope != task.Source+":read" && task.Scope != task.Source+":write" {
			return nil, fmt.Errorf("unsupported agent task %s", file)
		}
		ready := true
		for _, requirement := range task.Requires {
			ready = ready && done[requirement]
		}
		if ready {
			tasks = append(tasks, task)
		}
	}
	writers := 0
	for _, task := range tasks {
		if task.Source == "messages" {
			if task.Kind != "send" && task.Kind != "read" || task.Kind == "send" && task.Scope != "messages:write" || task.Kind == "read" && task.Scope != "messages:read" {
				return nil, fmt.Errorf("invalid Messages task")
			}
			if task.Kind == "send" {
				writers++
			}
		}
	}
	if writers > 1 {
		return nil, fmt.Errorf("at most one Messages send task is allowed per suite")
	}
	for source, requirement := range map[string]string{"reminders": "reminders.write", "contacts": "contacts.read", "calendar": "calendar.write", "messages": "messages.send", "notes": "notes.write", "usage": "usage.read"} {
		if !done[source+".read"] && !done[source+".write"] && !done[requirement] {
			continue
		}
		present := false
		for _, task := range tasks {
			present = present || task.Source == source
		}
		if !present {
			return nil, fmt.Errorf("done feature %s has no agent task", requirement)
		}
	}
	if done["messages.send"] && writers != 1 {
		return nil, fmt.Errorf("done Messages send feature has no send task")
	}
	if done["messages.read"] {
		present := false
		for _, task := range tasks {
			present = present || task.Source == "messages" && task.Kind == "read"
		}
		if !present {
			return nil, fmt.Errorf("done Messages read feature has no read task")
		}
	}
	if len(tasks) == 0 {
		return nil, fmt.Errorf("no tasks for done sources")
	}
	return tasks, nil
}

func (r AgentRunner) run(ctx context.Context, task Task, number int) (attempt Attempt) {
	if task.Source == "messages" {
		return r.runMessages(ctx, task, number)
	}
	start := time.Now()
	attempt.Number = number
	defer func() { attempt.Duration = time.Since(start).Milliseconds() }()
	ctx, cancel := context.WithTimeout(ctx, 6*time.Minute)
	defer cancel()
	w, err := world.NewReal(ctx, "replica", 1, world.BlobOptions{Scope: task.Scope + ",system:read"})
	if err != nil {
		attempt.Failure = "world_start"
		return
	}
	defer w.Close()
	diagnostics := func() {
		if attempt.Passed {
			return
		}
		path := filepath.Join(r.Private, fmt.Sprintf("%s-%d", task.ID, number))
		os.MkdirAll(path, 0700)
		for name, endpoint := range map[string]world.Endpoint{"gateway": w.Gateway, "replica": w.Replicas[0]} {
			records, err := w.Records(endpoint)
			if err == nil {
				raw, _ := json.Marshal(records)
				os.WriteFile(filepath.Join(path, name+".json"), raw, 0600)
			}
		}
	}
	tag := "[ab-test " + contracts.NewID("agent") + "]"
	createdList := ""
	defer func() {
		diagnostics()
		_, err := w.RecoverTruth("fixture", map[string]any{"source": task.Source, "action": "cleanup", "tag": tag})
		state, readErr := w.RecoverTruth("observe", map[string]any{"source": task.Source, "tag": tag})
		attempt.Cleanup = err == nil && readErr == nil && len(truthRecords(state)) == 0
		if createdList != "" {
			_, err = w.RecoverTruth("fixture", map[string]any{"source": "reminders", "action": "cleanup_list", "tag": tag, "list_id": createdList})
			attempt.Cleanup = attempt.Cleanup && err == nil
		}
		attempt.Passed = attempt.Passed && attempt.Cleanup
	}()
	if task.Source == "contacts" {
		_, err = w.Truth("fixture", map[string]any{"source": "contacts", "action": "create", "tag": tag})
		if err == nil {
			_, err = w.Truth("fixture", map[string]any{"source": "contacts", "action": "modify", "tag": tag, "job_title": "Bridge " + contracts.NewID("job")})
		}
	} else if task.Source == "calendar" {
		var state map[string]any
		state, err = w.Truth("observe", map[string]any{"source": "calendar", "tag": tag})
		calendars, _ := state["calendars"].([]any)
		if err == nil && len(calendars) == 0 {
			err = fmt.Errorf("existing Bridge Test calendar required")
		}
	} else {
		_, err = w.Truth("observe", map[string]any{"source": "reminders", "tag": tag})
		if err == nil {
			var fixture map[string]any
			fixture, err = w.Truth("fixture", map[string]any{"source": "reminders", "action": "create_list", "tag": tag})
			if fixture["created"] == true {
				createdList, _ = fixture["id"].(string)
			}
		}
	}
	if err != nil {
		attempt.Failure = "fixture"
		return
	}
	endpoint := w.Replicas[0]
	if err = w.Await(20*time.Second, func() bool {
		records, err := w.Records(endpoint)
		if err != nil {
			return false
		}
		for _, record := range records {
			if task.Source == "contacts" && bytes.Contains(record.Data, []byte(tag)) || record.Collection == map[string]string{"reminders": "reminders.list", "calendar": "calendar.calendar"}[task.Source] && bytes.Contains(record.Data, []byte("Bridge Test")) {
				return true
			}
		}
		return false
	}); err != nil {
		attempt.Failure = "fixture_replication"
		return
	}
	now := time.Now()
	tomorrow := now.AddDate(0, 0, 1)
	due := time.Date(tomorrow.Year(), tomorrow.Month(), tomorrow.Day(), 9, 0, 0, 0, now.Location())
	prompt := "Use only the apple_bridge MCP tools. Never use shell, file, browser, or other tools. Only modify data bearing this test tag: " + tag + ". Today is " + now.Format(time.RFC3339) + " in " + now.Location().String() + ".\n" + strings.ReplaceAll(task.Prompt, "{{tag}}", tag)
	result, err := r.execute(ctx, w, task, number, prompt, nil)
	if err != nil {
		attempt.Failure = "codex_execution"
		return
	}
	attempt.MCPOnly = result.Exit == 0 && result.Calls > 0 && result.Forbidden == 0
	state, err := w.Truth("observe", map[string]any{"source": task.Source, "tag": tag})
	records := truthRecords(state)
	if err != nil || len(records) != 1 {
		attempt.Failure = "truth_unique_record"
		return
	}
	if task.Source == "contacts" {
		answer, _ := records[0]["job_title"].(string)
		attempt.Truth = answer != "" && strings.TrimSpace(result.Answer) == answer
	} else if task.Source == "calendar" {
		title, _ := records[0]["title"].(string)
		attempt.Truth = strings.Contains(strings.ToLower(title), "test appointment") && sameTime(records[0]["start"], due) && sameTime(records[0]["end"], due.Add(30*time.Minute)) && records[0]["all_day"] == false
	} else {
		title, _ := records[0]["title"].(string)
		fields, _ := records[0]["due"].(map[string]any)
		attempt.Truth = strings.Contains(strings.ToLower(title), "buy milk") && fields["year"] == json.Number(fmt.Sprint(due.Year())) && fields["month"] == json.Number(fmt.Sprint(int(due.Month()))) && fields["day"] == json.Number(fmt.Sprint(due.Day())) && fields["hour"] == json.Number("9") && fields["minute"] == json.Number("0") && records[0]["completed"] == false
	}
	attempt.Replica = w.Await(15*time.Second, func() bool {
		actual, err := w.Records(endpoint)
		if err != nil {
			return false
		}
		count := 0
		for _, record := range actual {
			if record.Collection != task.Source+"."+map[string]string{"contacts": "contact", "reminders": "reminder", "calendar": "event"}[task.Source] {
				continue
			}
			var fields map[string]any
			if json.Unmarshal(record.Data, &fields) != nil {
				continue
			}
			if task.Source == "contacts" {
				if fields["given_name"] == records[0]["given_name"] && fields["job_title"] == records[0]["job_title"] && fields["organization"] == "Bridge Test" {
					count++
				}
			} else if task.Source == "calendar" {
				if fields["title"] == records[0]["title"] && sameTime(fields["start"], due) && sameTime(fields["end"], due.Add(30*time.Minute)) && fields["all_day"] == false {
					count++
				}
			} else if fields["title"] == records[0]["title"] {
				if value, ok := fields["due"].(string); ok {
					parsed, err := time.Parse(time.RFC3339, value)
					if err == nil && parsed.Equal(due) {
						count++
					}
				}
			}
		}
		return count == 1
	}) == nil
	attempt.Passed = attempt.MCPOnly && attempt.Truth && attempt.Replica
	return
}

func truthRecords(state map[string]any) []map[string]any {
	raw, _ := state["records"].([]any)
	records := []map[string]any{}
	for _, value := range raw {
		if record, ok := value.(map[string]any); ok {
			records = append(records, record)
		}
	}
	return records
}
func shellQuote(value string) string { return "'" + strings.ReplaceAll(value, "'", "'\"'\"'") + "'" }

func sameTime(value any, expected time.Time) bool {
	text, ok := value.(string)
	if !ok {
		return false
	}
	parsed, err := time.Parse(time.RFC3339Nano, text)
	return err == nil && parsed.Equal(expected)
}

func (r AgentRunner) execute(ctx context.Context, w *world.World, task Task, number int, prompt string, enabled []string) (result outcome, err error) {
	endpoint := w.Replicas[0]
	token, err := w.ScopedToken(endpoint, task.Scope)
	if err != nil {
		return result, err
	}
	directory := filepath.Join(w.Dir, "agent")
	request, _ := json.Marshal(map[string]any{"directory": directory, "url": "http://" + endpoint.Host + "/mcp", "token": token, "prompt": prompt, "tools": enabled})
	command := exec.CommandContext(ctx, "ssh", "ubuntu", "python3 -c "+shellQuote(string(r.Driver)))
	command.Stdin = bytes.NewReader(request)
	output, err := command.Output()
	private := filepath.Join(r.Private, fmt.Sprintf("%s-%d", task.ID, number))
	os.MkdirAll(private, 0700)
	exec.Command("scp", "-q", "ubuntu:"+directory+"/transcript.jsonl", "ubuntu:"+directory+"/stderr.log", private+"/").Run()
	if err != nil {
		return result, err
	}
	err = json.Unmarshal(output, &result)
	return
}
