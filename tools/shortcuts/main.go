package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"encoding/xml"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

type workflow struct {
	Version int      `json:"version"`
	Name    string   `json:"name"`
	Actions []action `json:"actions"`
}

type action struct {
	Key        string         `json:"key"`
	Identifier string         `json:"identifier"`
	Parameters map[string]any `json:"parameters"`
}

type artifact struct {
	Source   string `json:"source"`
	Unsigned string `json:"unsigned"`
	Signed   string `json:"signed,omitempty"`
	Mode     string `json:"mode"`
	SHA256   string `json:"unsigned_sha256"`
}

var safeName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9_-]*$`)

func stableID(name string) string {
	h := sha256.Sum256([]byte(name))
	return strings.ToUpper(fmt.Sprintf("%x-%x-%x-%x-%x", h[:4], h[4:6], h[6:8], h[8:10], h[10:16]))
}

func resolve(value any, name string, outputs map[string]string) (any, error) {
	switch v := value.(type) {
	case map[string]any:
		referenceFields := 1
		if v["$property"] != nil {
			referenceFields++
		}
		if v["$text"] == true {
			referenceFields++
		}
		if v["$coerce"] != nil {
			referenceFields++
		}
		if len(v) == referenceFields {
			if key, ok := v["$output"].(string); ok {
				id, exists := outputs[key]
				if !exists {
					return nil, fmt.Errorf("unknown or forward output %q", key)
				}
				value := map[string]any{"Type": "ActionOutput", "OutputUUID": id, "OutputName": key}
				if property, ok := v["$property"].(string); ok && property != "" {
					value["Aggrandizements"] = []any{map[string]any{"Type": "WFPropertyVariableAggrandizement", "PropertyName": property, "PropertyUserInfo": property}}
				}
				if coercion, ok := v["$coerce"].(string); ok && coercion != "" {
					aggrandizements, _ := value["Aggrandizements"].([]any)
					value["Aggrandizements"] = append(aggrandizements, map[string]any{"Type": "WFCoercionVariableAggrandizement", "CoercionItemClass": coercion})
				}
				if v["$text"] == true {
					return map[string]any{"Value": map[string]any{"string": "\ufffc", "attachmentsByRange": map[string]any{"{0, 1}": value}}, "WFSerializationType": "WFTextTokenString"}, nil
				}
				return map[string]any{"Value": value, "WFSerializationType": "WFTextTokenAttachment"}, nil
			}
			if group, ok := v["$group"].(string); ok && group != "" {
				return stableID(name + "/group/" + group), nil
			}
			if input, ok := v["$input"].(bool); ok && input {
				return map[string]any{"Value": map[string]any{"Type": "ExtensionInput"}, "WFSerializationType": "WFTextTokenAttachment"}, nil
			}
		}
		out := map[string]any{}
		for k, item := range v {
			if strings.HasPrefix(k, "$") {
				return nil, fmt.Errorf("invalid reference %q", k)
			}
			converted, err := resolve(item, name, outputs)
			if err != nil {
				return nil, err
			}
			out[k] = converted
		}
		return out, nil
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			converted, err := resolve(item, name, outputs)
			if err != nil {
				return nil, err
			}
			out[i] = converted
		}
		return out, nil
	default:
		return value, nil
	}
}

func plistValue(b *bytes.Buffer, value any) error {
	text := func(tag, value string) {
		b.WriteString("<" + tag + ">")
		_ = xml.EscapeText(b, []byte(value))
		b.WriteString("</" + tag + ">\n")
	}
	switch v := value.(type) {
	case string:
		text("string", v)
	case json.Number:
		if _, err := v.Int64(); err == nil {
			text("integer", v.String())
		} else if n, err := v.Float64(); err == nil {
			text("real", strconv.FormatFloat(n, 'g', -1, 64))
		} else {
			return fmt.Errorf("invalid number")
		}
	case int:
		text("integer", strconv.Itoa(v))
	case bool:
		if v {
			b.WriteString("<true/>\n")
		} else {
			b.WriteString("<false/>\n")
		}
	case []any:
		b.WriteString("<array>\n")
		for _, item := range v {
			if err := plistValue(b, item); err != nil {
				return err
			}
		}
		b.WriteString("</array>\n")
	case map[string]any:
		b.WriteString("<dict>\n")
		keys := make([]string, 0, len(v))
		for k := range v {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			text("key", k)
			if err := plistValue(b, v[k]); err != nil {
				return err
			}
		}
		b.WriteString("</dict>\n")
	default:
		return fmt.Errorf("unsupported plist value %T", value)
	}
	return nil
}

func compile(path string) ([]byte, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	decoder.UseNumber()
	var source workflow
	if err = decoder.Decode(&source); err != nil {
		return nil, err
	}
	var trailing any
	if err = decoder.Decode(&trailing); err != io.EOF {
		return nil, fmt.Errorf("expected one JSON document")
	}
	if source.Version != 1 || strings.TrimSpace(source.Name) == "" || source.Actions == nil {
		return nil, fmt.Errorf("version 1, name and actions required")
	}
	outputs := map[string]string{}
	actions := make([]any, 0, len(source.Actions))
	for _, a := range source.Actions {
		if !safeName.MatchString(a.Key) || a.Identifier == "" || outputs[a.Key] != "" {
			return nil, fmt.Errorf("unique action key and identifier required")
		}
		if _, exists := a.Parameters["UUID"]; exists {
			return nil, fmt.Errorf("UUID is generated from action key")
		}
		params := a.Parameters
		if params == nil {
			params = map[string]any{}
		}
		converted, err := resolve(params, source.Name, outputs)
		if err != nil {
			return nil, err
		}
		id := stableID(source.Name + "/action/" + a.Key)
		converted.(map[string]any)["UUID"] = id
		outputs[a.Key] = id
		actions = append(actions, map[string]any{"WFWorkflowActionIdentifier": a.Identifier, "WFWorkflowActionParameters": converted})
	}
	plist := map[string]any{"WFWorkflowName": source.Name, "WFWorkflowActions": actions, "WFWorkflowClientVersion": "2600.0.0", "WFWorkflowMinimumClientVersion": 900, "WFWorkflowMinimumClientVersionString": "900", "WFWorkflowIcon": map[string]any{"WFWorkflowIconStartColor": 4282601983, "WFWorkflowIconGlyphNumber": 59511}, "WFWorkflowTypes": []any{}, "WFWorkflowInputContentItemClasses": []any{"WFStringContentItem", "WFDictionaryContentItem"}, "WFWorkflowImportQuestions": []any{}}
	var b bytes.Buffer
	b.WriteString("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n")
	if err = plistValue(&b, plist); err != nil {
		return nil, err
	}
	b.WriteString("</plist>\n")
	return b.Bytes(), nil
}

func sign(ctx context.Context, input, output, mode string) (string, error) {
	modes := []string{mode}
	if mode == "auto" {
		modes = []string{"anyone", "people-who-know-me"}
	}
	for _, candidate := range modes {
		_ = os.Remove(output)
		err := exec.CommandContext(ctx, "/usr/bin/shortcuts", "sign", "--mode", candidate, "--input", input, "--output", output).Run()
		if err == nil {
			data, err := os.ReadFile(output)
			if err != nil || len(data) < 4 || string(data[:4]) != "AEA1" {
				_ = os.Remove(output)
				return "", fmt.Errorf("signer did not produce an AEA archive")
			}
			return candidate, nil
		}
		if ctx.Err() != nil {
			_ = os.Remove(output)
			return "", ctx.Err()
		}
	}
	_ = os.Remove(output)
	return "", fmt.Errorf("shortcuts sign failed in requested mode(s): %s", strings.Join(modes, ", "))
}

func run(ctx context.Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: shortcuts build [-source file.json] [-out directory] [-unsigned] [-mode auto|anyone|people-who-know-me] | open file.shortcut")
	}
	if args[0] == "open" {
		if len(args) != 2 || filepath.Ext(args[1]) != ".shortcut" {
			return fmt.Errorf("open requires one signed .shortcut file")
		}
		path, err := filepath.Abs(args[1])
		if err != nil {
			return err
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if len(raw) < 4 || string(raw[:4]) != "AEA1" {
			return fmt.Errorf("expected a signed AEA shortcut")
		}
		return exec.CommandContext(ctx, "/usr/bin/open", path).Run()
	}
	if args[0] != "build" {
		return fmt.Errorf("unknown command %q", args[0])
	}
	flags := flag.NewFlagSet("build", flag.ContinueOnError)
	source := flags.String("source", "", "single JSON source (default: packaging/shortcuts/*.json)")
	out := flags.String("out", ".build/shortcuts", "output directory")
	unsigned := flags.Bool("unsigned", false, "compile only, without macOS signing")
	mode := flags.String("mode", "auto", "auto tries anyone then people-who-know-me")
	if err := flags.Parse(args[1:]); err != nil {
		return err
	}
	if flags.NArg() != 0 || (*mode != "auto" && *mode != "anyone" && *mode != "people-who-know-me") {
		return fmt.Errorf("invalid build arguments or signing mode")
	}
	paths := []string{*source}
	if *source == "" {
		var err error
		paths, err = filepath.Glob("packaging/shortcuts/*.json")
		if err != nil {
			return err
		}
	}
	if len(paths) == 0 {
		return fmt.Errorf("no shortcut sources")
	}
	if err := os.MkdirAll(*out, 0755); err != nil {
		return err
	}
	if err := os.Remove(filepath.Join(*out, "manifest.json")); err != nil && !os.IsNotExist(err) {
		return err
	}
	artifacts := []artifact{}
	seenNames := map[string]bool{}
	for _, path := range paths {
		stem := strings.TrimSuffix(filepath.Base(path), ".json")
		if !safeName.MatchString(stem) {
			return fmt.Errorf("invalid source filename")
		}
		data, err := compile(path)
		if err != nil {
			return fmt.Errorf("%s: %w", path, err)
		}
		sourceBytes, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		var display workflow
		if err = json.Unmarshal(sourceBytes, &display); err != nil {
			return err
		}
		if strings.TrimSpace(display.Name) != display.Name || strings.ContainsAny(display.Name, "/\\\x00") || display.Name == "." || display.Name == ".." {
			return fmt.Errorf("invalid display name")
		}
		if seenNames[display.Name] {
			return fmt.Errorf("duplicate display name")
		}
		seenNames[display.Name] = true
		a := artifact{Source: path, Unsigned: filepath.Join(*out, stem+".unsigned.shortcut"), Mode: "unsigned", SHA256: fmt.Sprintf("%x", sha256.Sum256(data))}
		if err = os.WriteFile(a.Unsigned, data, 0644); err != nil {
			return err
		}
		if !*unsigned {
			a.Signed = filepath.Join(*out, display.Name+".shortcut")
			a.Mode, err = sign(ctx, a.Unsigned, a.Signed, *mode)
			if err != nil {
				return err
			}
		}
		artifacts = append(artifacts, a)
	}
	raw, err := json.MarshalIndent(artifacts, "", "  ")
	if err != nil {
		return err
	}
	raw = append(raw, '\n')
	if err = os.WriteFile(filepath.Join(*out, "manifest.json"), raw, 0644); err != nil {
		return err
	}
	_, err = os.Stdout.Write(raw)
	return err
}

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	if err := run(ctx, os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
