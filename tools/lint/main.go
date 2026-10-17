package main

import (
	"fmt"
	"go/scanner"
	"go/token"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var skipDirs = map[string]bool{".git": true, ".build": true, "bin": true, "node_modules": true, "evidence": true, "fixtures": true, "testdata": true}

var importRules = map[string][]string{
	"internal/contracts":   {"internal/"},
	"internal/trust":       {"internal/state", "internal/operations", "internal/replication", "internal/blobs", "internal/frontdoor", "internal/runtime"},
	"internal/state":       {"internal/operations", "internal/replication", "internal/blobs", "internal/frontdoor", "internal/runtime"},
	"internal/blobs":       {"internal/state", "internal/operations", "internal/replication", "internal/frontdoor", "internal/runtime"},
	"internal/operations":  {"internal/replication", "internal/frontdoor", "internal/runtime"},
	"internal/replication": {"internal/operations", "internal/frontdoor", "internal/runtime"},
	"internal/frontdoor":   {"internal/state", "internal/runtime", "internal/applepeer"},
	"internal/":            {"/lab"},
}

type finding struct {
	path string
	line int
	msg  string
}

func main() {
	root := "."
	if len(os.Args) > 1 {
		root = os.Args[1]
	}
	var findings []finding
	_ = filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			if skipDirs[d.Name()] && path != root {
				return filepath.SkipDir
			}
			return nil
		}
		rel, _ := filepath.Rel(root, path)
		switch {
		case strings.HasSuffix(path, ".go"):
			findings = append(findings, lintGo(rel, path)...)
		case strings.HasSuffix(path, ".swift"):
			findings = append(findings, lintSwift(rel, path)...)
		}
		return nil
	})
	for _, f := range findings {
		fmt.Printf("%s:%d: %s\n", f.path, f.line, f.msg)
	}
	if len(findings) > 0 {
		fmt.Printf("lint: %d problem(s)\n", len(findings))
		os.Exit(1)
	}
	fmt.Println("lint: ok")
}

func lintGo(rel, path string) []finding {
	src, err := os.ReadFile(path)
	if err != nil {
		return []finding{{rel, 0, err.Error()}}
	}
	var out []finding
	fset := token.NewFileSet()
	file := fset.AddFile(path, fset.Base(), len(src))
	var s scanner.Scanner
	s.Init(file, src, nil, scanner.ScanComments)
	importing := false
	for {
		pos, tok, lit := s.Scan()
		if tok == token.EOF {
			break
		}
		line := fset.Position(pos).Line
		switch {
		case tok == token.COMMENT && strings.HasPrefix(lit, "//go:build "):
		case tok == token.COMMENT:
			out = append(out, finding{rel, line, "comment"})
		case tok == token.IMPORT:
			importing = true
		case importing && tok == token.STRING:
			if msg := checkImport(rel, strings.Trim(lit, "\"`")); msg != "" {
				out = append(out, finding{rel, line, msg})
			}
		case importing && (tok == token.RPAREN || tok == token.FUNC || tok == token.TYPE || tok == token.VAR || tok == token.CONST):
			importing = false
		}
	}
	return out
}

func checkImport(rel, imp string) string {
	const module = "github.com/JohanSardar/apple-bridge/"
	if !strings.HasPrefix(imp, module) {
		return ""
	}
	target := strings.TrimPrefix(imp, module)
	dir := filepath.ToSlash(filepath.Dir(rel))
	if strings.HasPrefix(dir, "internal/contracts") && !strings.HasPrefix(target, "internal/contracts") {
		return "contracts must not import " + target
	}
	for prefix, banned := range importRules {
		if prefix == "internal/contracts" || !strings.HasPrefix(dir, prefix) {
			continue
		}
		for _, b := range banned {
			if b == "/lab" {
				if strings.HasPrefix(target, "lab") {
					return "product code must not import " + target
				}
				continue
			}
			if strings.HasPrefix(target, b) && !strings.HasPrefix(dir, b) {
				return dir + " must not import " + target
			}
		}
	}
	return ""
}

func lintSwift(rel, path string) []finding {
	src, err := os.ReadFile(path)
	if err != nil {
		return []finding{{rel, 0, err.Error()}}
	}
	var out []finding
	line := 1
	n := len(src)
	for i := 0; i < n; i++ {
		c := src[i]
		switch {
		case c == '\n':
			line++
		case c == '"':
			i, line = skipSwiftString(src, i, line)
		case c == '/' && i+1 < n && src[i+1] == '/':
			end := i
			for end < n && src[end] != '\n' {
				end++
			}
			if !(i == 0 && line == 1 && filepath.Base(path) == "Package.swift" && regexp.MustCompile(`^// swift-tools-version:[0-9.]+$`).Match(src[i:end])) {
				out = append(out, finding{rel, line, "comment"})
			}
			for i < n && src[i] != '\n' {
				i++
			}
			line++
		case c == '/' && i+1 < n && src[i+1] == '*':
			out = append(out, finding{rel, line, "comment"})
			i += 2
			for i+1 < n && !(src[i] == '*' && src[i+1] == '/') {
				if src[i] == '\n' {
					line++
				}
				i++
			}
			i++
		}
	}
	return out
}

func skipSwiftString(src []byte, i, line int) (int, int) {
	n := len(src)
	if i+2 < n && src[i+1] == '"' && src[i+2] == '"' {
		i += 3
		for i+2 < n && !(src[i] == '"' && src[i+1] == '"' && src[i+2] == '"') {
			if src[i] == '\n' {
				line++
			}
			i++
		}
		return i + 2, line
	}
	i++
	for i < n && src[i] != '"' {
		if src[i] == '\\' {
			i++
		}
		if i < n && src[i] == '\n' {
			line++
		}
		i++
	}
	return i, line
}
