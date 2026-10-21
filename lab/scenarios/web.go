package scenarios

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type WebOptions struct {
	Browsers []string `yaml:"browsers"`
	Themes   []string `yaml:"themes"`
	Layouts  []string `yaml:"layouts"`
	Server   string   `yaml:"server"`
}

func validateWeb(s Scenario) error {
	if !slices.Contains(s.Levels, "web") || s.Web == nil || len(s.Web.Browsers)*len(s.Web.Themes)*len(s.Web.Layouts) == 0 || (s.Web.Server != "mock" && s.Web.Server != "gateway") {
		return fmt.Errorf("web scenario requires matrix and server")
	}
	for _, pair := range []struct{ values, allowed []string }{{s.Web.Browsers, []string{"webkit", "chromium", "firefox"}}, {s.Web.Themes, []string{"light", "dark"}}, {s.Web.Layouts, []string{"phone", "desktop"}}} {
		for _, value := range pair.values {
			if !slices.Contains(pair.allowed, value) {
				return fmt.Errorf("invalid web matrix value %q", value)
			}
		}
	}
	return nil
}

func runWeb(ctx context.Context, s Scenario) Evidence {
	if err := validateWeb(s); err != nil {
		return runWebProjects(ctx, s, nil)
	}
	projects := []string{}
	for _, browser := range s.Web.Browsers {
		for _, theme := range s.Web.Themes {
			for _, layout := range s.Web.Layouts {
				projects = append(projects, browser+"-"+theme+"-"+layout)
			}
		}
	}

	if s.Web == nil || s.Web.Server != "gateway" {
		return runWebProjects(ctx, s, projects)
	}
	var result Evidence
	reports := []any{}
	for _, project := range projects {
		part := runWebProjects(ctx, s, []string{project})
		if result.RunID == "" {
			result = part
			result.Assertions = nil
		}
		for _, assertion := range part.Assertions {
			assertion.Name = project + ":" + assertion.Name
			result.Assertions = append(result.Assertions, assertion)
		}
		reports = append(reports, part.Metrics["report"])
		result.Passed = part.Passed
		if !part.Passed {
			break
		}
	}
	result.Metrics["matrix"] = projects
	result.Metrics["reports"] = reports
	return result
}

func runWebProjects(ctx context.Context, s Scenario, projects []string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: "web", GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}, Assertions: []Assertion{}, Latency: map[string]Latency{}}
	fail := func(name string, err error) Evidence {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Detail: fmt.Sprint(err)})
		return e
	}
	if err := validateWeb(s); err != nil {
		return fail("web_configuration", err)
	}
	w, err := world.New(ctx, "sim", 0, 4, world.BlobOptions{Scope: "admin"})
	if err != nil {
		return fail("web_world", err)
	}
	defer w.Close()
	endpoint := w.Gateway.URL
	controlURL, controlKey := "", ""
	if s.Web.Server == "gateway" {
		parsed, err := url.Parse(endpoint)
		if err != nil {
			return fail("web_origin", err)
		}
		parsed.Host = "localhost:" + parsed.Port()
		endpoint = parsed.String()
		if err := w.ControlAction("/messages", map[string]any{"action": "seed", "count": 4}); err != nil {
			return fail("web_seed", err)
		}
		control, key := webBrowserControl(ctx, w, endpoint)
		defer control.Close()
		controlURL, controlKey = control.URL, key
	}
	if s.Web.Server == "mock" {
		ready := filepath.Join(w.Dir, "web-ready")
		server := exec.CommandContext(ctx, "pnpm", "exec", "node", "dev/serve.mjs")
		server.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
		server.Cancel = func() error { return syscall.Kill(-server.Process.Pid, syscall.SIGKILL) }
		server.WaitDelay = 5 * time.Second
		server.Dir = "web"
		server.Env = append(os.Environ(), "AB_WEB_MOCK=1", "AB_WEB_READY="+ready, "AB_WEB_WORLD="+w.Gateway.URL)
		log, err := os.Create(filepath.Join(w.Dir, "web.log"))
		if err != nil {
			return fail("web_log", err)
		}
		defer log.Close()
		server.Stdout, server.Stderr = log, log
		if err := server.Start(); err != nil {
			return fail("web_server", err)
		}
		defer func() { syscall.Kill(-server.Process.Pid, syscall.SIGKILL); server.Wait() }()
		if err := w.Await(15*time.Second, func() bool {
			b, err := os.ReadFile(ready)
			if err == nil {
				endpoint = string(b)
			}
			return err == nil
		}); err != nil {
			output, _ := os.ReadFile(log.Name())
			return fail("web_ready", fmt.Errorf("%w: %s", err, output))
		}
	}
	if err := os.MkdirAll("lab/evidence/private/web/reports", 0700); err != nil {
		return fail("web_evidence", err)
	}
	suffix := "matrix"
	if len(projects) == 1 {
		suffix = projects[0]
	}
	report, _ := filepath.Abs(filepath.Join("lab/evidence/private/web/reports", s.ID+"-"+suffix+"-playwright.json"))
	test := exec.CommandContext(ctx, "pnpm", "exec", "playwright", "test", "-g", "\\b"+regexp.QuoteMeta(s.ID)+"\\b")
	test.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	test.Cancel = func() error { return syscall.Kill(-test.Process.Pid, syscall.SIGKILL) }
	test.WaitDelay = 5 * time.Second
	test.Dir = "web"
	test.Env = append(os.Environ(), "AB_WEB_CONTROL="+controlURL, "AB_WEB_CONTROL_KEY="+controlKey, "AB_WEB_URL="+endpoint, "AB_WEB_PROJECTS="+strings.Join(projects, ","), "AB_WEB_REPORT="+report, "AB_WEB_SERVER="+s.Web.Server, "AB_WEB_WORLD="+w.Gateway.URL)
	output, err := test.CombinedOutput()
	e.Metrics["matrix"] = projects
	e.Metrics["server"] = s.Web.Server
	e.Metrics["report"] = "lab/evidence/private/web/reports/" + filepath.Base(report)
	detail := string(output)
	if err != nil {
		logPath := strings.TrimSuffix(report, ".json") + ".log"
		if writeErr := os.WriteFile(logPath, output, 0600); writeErr != nil {
			return fail("web_private_log", writeErr)
		}
		detail = fmt.Sprintf("Playwright failed: %v; private log: %s", err, filepath.Base(logPath))
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "playwright_matrix", Passed: err == nil, Detail: detail})
	e.Passed = err == nil
	return e
}
