package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"html/template"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

type scenarioRef struct {
	ID     string   `yaml:"id"`
	Levels []string `yaml:"levels"`
}

type limit struct {
	Scenario string `yaml:"scenario"`
	Level    string `yaml:"level"`
	Reason   string `yaml:"reason"`
}

type feature struct {
	Notes        string        `yaml:"notes"`
	Limits       []limit       `yaml:"limits"`
	ID           string        `yaml:"id"`
	Title        string        `yaml:"title"`
	Requirements []string      `yaml:"requirements"`
	Milestone    string        `yaml:"milestone"`
	Modules      []string      `yaml:"modules"`
	DependsOn    []string      `yaml:"depends_on"`
	Status       string        `yaml:"status"`
	Scenarios    []scenarioRef `yaml:"scenarios"`
}

type graph struct {
	Version      int               `yaml:"version"`
	Levels       map[string]string `yaml:"levels"`
	Statuses     []string          `yaml:"statuses"`
	StatusRules  map[string]string `yaml:"status_rules"`
	Requirements map[string]string `yaml:"requirements"`
	Features     []feature         `yaml:"features"`
}

type evidence struct {
	Scenario string `json:"scenario"`
	Level    string `json:"level"`
	Passed   bool   `json:"passed"`
	GitSHA   string `json:"git_sha"`
	At       string `json:"at"`
}

type levelResult struct {
	Level  string
	State  string
	GitSHA string
	At     string
}

type scenarioView struct {
	ID      string
	File    string
	HasFile bool
	Results []levelResult
}

type featureView struct {
	feature
	Views    []scenarioView
	Problems []string
}

type checker struct {
	root     string
	graph    graph
	problems []string
}

func main() {
	root := flag.String("root", ".", "repository root")
	htmlOut := flag.String("html", "", "write an HTML render to this path")
	flag.Parse()
	c, err := load(*root)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	views := c.check()
	if *htmlOut != "" {
		if err := render(*htmlOut, c.graph, views); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
	}
	summarize(views)
	if len(c.problems) > 0 {
		for _, p := range c.problems {
			fmt.Fprintln(os.Stderr, "problem:", p)
		}
		os.Exit(1)
	}
}

func load(root string) (*checker, error) {
	raw, err := os.ReadFile(filepath.Join(root, "features.yaml"))
	if err != nil {
		return nil, err
	}
	var g graph
	dec := yaml.NewDecoder(strings.NewReader(string(raw)))
	dec.KnownFields(true)
	if err := dec.Decode(&g); err != nil {
		return nil, fmt.Errorf("features.yaml: %w", err)
	}
	return &checker{root: root, graph: g}, nil
}

func (c *checker) fail(format string, args ...any) {
	c.problems = append(c.problems, fmt.Sprintf(format, args...))
}

func (c *checker) check() []featureView {
	scenarioFiles := c.indexScenarioFiles()
	ids := map[string]bool{}
	owner := map[string]string{}
	statuses := map[string]bool{}
	for _, s := range c.graph.Statuses {
		statuses[s] = true
	}
	for _, f := range c.graph.Features {
		if ids[f.ID] {
			c.fail("feature %s defined twice", f.ID)
		}
		ids[f.ID] = true
	}
	var views []featureView
	for _, f := range c.graph.Features {
		v := featureView{feature: f}
		if !statuses[f.Status] {
			c.fail("%s: unknown status %q", f.ID, f.Status)
		}
		if len(f.Scenarios) == 0 {
			c.fail("%s: no scenarios", f.ID)
		}
		if len(f.Requirements) == 0 {
			c.fail("%s: no requirements", f.ID)
		}
		for _, r := range f.Requirements {
			if _, ok := c.graph.Requirements[r]; !ok {
				c.fail("%s: unknown requirement %s", f.ID, r)
			}
		}
		for _, d := range f.DependsOn {
			if !ids[d] {
				c.fail("%s: depends on unknown feature %s", f.ID, d)
			}
		}
		for _, s := range f.Scenarios {
			if prev, ok := owner[s.ID]; ok {
				c.fail("scenario %s owned by both %s and %s", s.ID, prev, f.ID)
			}
			owner[s.ID] = f.ID
			sv := scenarioView{ID: s.ID, File: scenarioFiles[s.ID]}
			sv.HasFile = sv.File != ""
			if !sv.HasFile && f.Status != "todo" {
				c.fail("%s: scenario %s has no file under lab/scenarios", f.ID, s.ID)
			}
			for _, l := range s.Levels {
				if _, ok := c.graph.Levels[l]; !ok {
					c.fail("%s: scenario %s has unknown level %s", f.ID, s.ID, l)
				}
				sv.Results = append(sv.Results, c.result(s.ID, l))
			}
			v.Views = append(v.Views, sv)
		}
		limited := map[string]bool{}
		for _, l := range f.Limits {
			if l.Reason == "" {
				c.fail("%s: limit on %s/%s has no reason", f.ID, l.Scenario, l.Level)
			}
			limited[l.Scenario+"/"+l.Level] = true
		}
		if f.Status == "limited" && len(f.Limits) == 0 {
			c.fail("%s is limited but lists no limits", f.ID)
		}
		if f.Status == "done" || f.Status == "limited" {
			for _, sv := range v.Views {
				for _, r := range sv.Results {
					if r.State != "passed" && !(f.Status == "limited" && limited[sv.ID+"/"+r.Level]) {
						c.fail("%s is %s but %s at %s is %s", f.ID, f.Status, sv.ID, r.Level, r.State)
					}
				}
			}
		}
		views = append(views, v)
	}
	for id, file := range scenarioFiles {
		if _, ok := owner[id]; !ok {
			c.fail("scenario file %s (%s) is not linked to any feature", file, id)
		}
	}
	c.checkAcyclic()
	return views
}

func (c *checker) indexScenarioFiles() map[string]string {
	files := map[string]string{}
	features := map[string]string{}
	levels := map[string]map[string]bool{}
	dir := filepath.Join(c.root, "lab", "scenarios")
	_ = filepath.WalkDir(dir, func(path string, d os.DirEntry, err error) error {
		if err != nil || d.IsDir() || !strings.HasSuffix(path, ".yaml") {
			return nil
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			return nil
		}
		var head struct {
			ID      string   `yaml:"id"`
			Feature string   `yaml:"feature"`
			Levels  []string `yaml:"levels"`
		}
		if err := yaml.Unmarshal(raw, &head); err != nil || head.ID == "" {
			c.fail("scenario file %s has no id", path)
			return nil
		}
		rel, _ := filepath.Rel(c.root, path)
		if prev, ok := files[head.ID]; ok {
			if features[head.ID] != head.Feature || len(head.Levels) == 0 {
				c.fail("scenario %s has incompatible definitions in %s and %s", head.ID, prev, rel)
			}
			for _, level := range head.Levels {
				if levels[head.ID][level] {
					c.fail("scenario %s at %s defined in %s and %s", head.ID, level, prev, rel)
				}
			}
			files[head.ID] = prev + ", " + rel
		} else {
			files[head.ID] = rel
			features[head.ID] = head.Feature
			levels[head.ID] = map[string]bool{}
		}
		for _, level := range head.Levels {
			levels[head.ID][level] = true
		}
		return nil
	})
	return files
}

func (c *checker) result(id, level string) levelResult {
	r := levelResult{Level: level, State: "missing"}
	raw, err := os.ReadFile(filepath.Join(c.root, "lab", "evidence", level, id+".json"))
	if err != nil {
		return r
	}
	var e evidence
	if err := json.Unmarshal(raw, &e); err != nil {
		r.State = "corrupt"
		return r
	}
	r.GitSHA, r.At = e.GitSHA, e.At
	if e.Passed {
		r.State = "passed"
	} else {
		r.State = "failed"
	}
	return r
}

func (c *checker) checkAcyclic() {
	deps := map[string][]string{}
	for _, f := range c.graph.Features {
		deps[f.ID] = f.DependsOn
	}
	state := map[string]int{}
	var visit func(string, []string)
	visit = func(id string, path []string) {
		switch state[id] {
		case 1:
			c.fail("dependency cycle: %s", strings.Join(append(path, id), " -> "))
			return
		case 2:
			return
		}
		state[id] = 1
		for _, d := range deps[id] {
			visit(d, append(path, id))
		}
		state[id] = 2
	}
	keys := make([]string, 0, len(deps))
	for k := range deps {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		visit(k, nil)
	}
}

func summarize(views []featureView) {
	counts := map[string]int{}
	passed, total := 0, 0
	for _, v := range views {
		counts[v.Status]++
		for _, s := range v.Views {
			for _, r := range s.Results {
				total++
				if r.State == "passed" {
					passed++
				}
			}
		}
	}
	fmt.Printf("features: %d (done %d, limited %d, in_progress %d, todo %d, blocked %d); scenario-levels passing %d/%d\n",
		len(views), counts["done"], counts["limited"], counts["in_progress"], counts["todo"], counts["blocked"], passed, total)
}

var page = template.Must(template.New("page").Parse(`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Apple Bridge feature graph</title>
<style>
body{font:14px/1.45 -apple-system,system-ui,sans-serif;margin:24px;color:#1d1d1f;background:#fbfbfd}
h1{font-size:20px}h2{font-size:15px;margin:0 0 4px}
.f{background:#fff;border:1px solid #e3e3e8;border-radius:10px;padding:12px 14px;margin:10px 0}
.meta{color:#6e6e73;font-size:12px}
.st{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;margin-left:6px}
.st-done{background:#d9f5df}.st-in_progress{background:#fff1c7}.st-todo{background:#eee}.st-blocked{background:#ffd9d6}.st-limited{background:#e5dcff}
table{border-collapse:collapse;margin-top:6px}td,th{padding:2px 10px 2px 0;font-size:12px;text-align:left}
.passed{color:#137333}.failed,.corrupt{color:#b3261e}.missing{color:#8e8e93}
</style></head><body>
<h1>Apple Bridge feature graph</h1>
{{range .Views}}<div class="f" id="{{.ID}}">
<h2>{{.ID}}<span class="st st-{{.Status}}">{{.Status}}</span></h2>
<div>{{.Title}}</div>
<div class="meta">{{.Milestone}} · requirements: {{range .Requirements}}{{.}} {{end}}· modules: {{range .Modules}}{{.}} {{end}}{{if .DependsOn}}· depends on: {{range .DependsOn}}<a href="#{{.}}">{{.}}</a> {{end}}{{end}}</div>
<table><tr><th>scenario</th><th>file</th><th>levels</th></tr>
{{range .Views}}<tr><td>{{.ID}}</td><td>{{if .HasFile}}{{.File}}{{else}}<span class="missing">not written</span>{{end}}</td>
<td>{{range .Results}}<span class="{{.State}}">{{.Level}}:{{.State}}</span> {{end}}</td></tr>{{end}}
</table></div>{{end}}
</body></html>`))

func render(path string, g graph, views []featureView) error {
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()
	return page.Execute(f, struct {
		Graph graph
		Views []featureView
	}{g, views})
}
