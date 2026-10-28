package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

func lintSourceRegistries(root string) []finding {
	var out []finding
	rules := []struct{ path, required, message string }{

		{"lab/simapple/simapple.go", "range sourceRoutes", "source routes must use the per-source registry"},
		{"lab/simapple/commands.go", "range sourceCommands", "source commands must use the per-source registry"},
		{"native/Package.swift", "sourceTargets.map", "AppleSession dependencies must derive from source target blocks"},
		{"native/Sources/AppleSession/HelperCommand.swift", "SessionSources.register(", "source registration belongs in SessionSources"},
	}
	for _, rule := range rules {
		data, err := os.ReadFile(filepath.Join(root, rule.path))
		if err != nil {
			out = append(out, finding{rule.path, 0, err.Error()})
			continue
		}
		if !strings.Contains(string(data), rule.required) {
			out = append(out, finding{rule.path, 1, rule.message})
		}
	}
	data, err := os.ReadFile(filepath.Join(root, "lab/simapple/simapple.go"))
	if err == nil {
		for _, match := range regexp.MustCompile(`fake\.([A-Za-z]+(?:Control|Routes))\(mux\)`).FindAllStringSubmatch(string(data), -1) {
			if match[1] != "commandControl" && match[1] != "blobRoutes" && match[1] != "datasetRoutes" {
				out = append(out, finding{"lab/simapple/simapple.go", 1, "source routes must register from their source file"})
			}
		}
	}

	for _, rel := range []string{"lab/simapple/simapple.go", "lab/simapple/commands.go", "native/Sources/AppleSession/HelperCommand.swift", "native/Sources/AppleSession/SessionSources.swift"} {
		if body, err := os.ReadFile(filepath.Join(root, rel)); err == nil {
			out = append(out, lintBootstrapRegistrations(rel, body)...)
		}
	}
	return out
}

func lintBootstrapRegistrations(rel string, src []byte) []finding {
	var out []finding
	if strings.HasSuffix(rel, ".swift") {
		body := regexp.MustCompile(`SessionSources\s*\.\s*register\s*\(`).ReplaceAll(src, nil)
		if regexp.MustCompile(`\.\s*register\s*\(`).Match(body) {
			out = append(out, finding{rel, 1, "source registration must live in native/Registrations/<target>.swift"})
		}
		return out
	}
	file, err := parser.ParseFile(token.NewFileSet(), rel, src, 0)
	if err != nil {
		return []finding{{rel, 1, err.Error()}}
	}
	ast.Inspect(file, func(node ast.Node) bool {
		switch n := node.(type) {
		case *ast.AssignStmt:
			for _, left := range n.Lhs {
				if id, ok := left.(*ast.Ident); ok && (id.Name == "sourceRoutes" || id.Name == "sourceCommands") {
					out = append(out, finding{rel, 1, "source registrations must live in per-source files"})
				}
			}
		case *ast.CallExpr:
			if selector, ok := n.Fun.(*ast.SelectorExpr); ok && (strings.HasPrefix(selector.Sel.Name, "route") || strings.HasSuffix(selector.Sel.Name, "Command")) {
				out = append(out, finding{rel, 1, "source handlers must use the per-source registry"})
			}
		}
		return true
	})
	return out
}

func lintDescriptorFile(rel string, src []byte) []finding {
	if !strings.HasPrefix(rel, "internal/contracts/") || strings.HasSuffix(rel, "_test.go") {
		return nil
	}
	file, err := parser.ParseFile(token.NewFileSet(), rel, src, 0)
	if err != nil {
		return []finding{{rel, 1, err.Error()}}
	}
	var out []finding
	hasDescriptors := false
	ast.Inspect(file, func(node ast.Node) bool {
		literal, ok := node.(*ast.CompositeLit)
		if !ok || len(literal.Elts) == 0 {
			return true
		}
		if kind, ok := literal.Type.(*ast.Ident); ok && kind.Name == "OperationDescriptor" {
			hasDescriptors = true
		}
		if array, ok := literal.Type.(*ast.ArrayType); ok {
			if kind, ok := array.Elt.(*ast.Ident); ok && kind.Name == "OperationDescriptor" {
				hasDescriptors = true
			}
		}
		if !hasDescriptors {
			return true
		}
		for _, entry := range literal.Elts {
			pair, ok := entry.(*ast.KeyValueExpr)
			if !ok {
				continue
			}
			key, ok := pair.Key.(*ast.Ident)
			if !ok || key.Name != "Source" {
				continue
			}
			source, ok := pair.Value.(*ast.Ident)
			if ok && filepath.Base(rel) != strings.ToLower(source.Name)+"_descriptors.go" {
				out = append(out, finding{rel, 1, "source descriptors must live in <source>_descriptors.go"})
			}
		}
		return true
	})
	if hasDescriptors && (!strings.HasSuffix(rel, "_descriptors.go") || !strings.Contains(string(src), "registerDescriptors(")) {
		out = append(out, finding{rel, 1, "descriptor files must register their per-source function"})
	}
	if rel != "internal/contracts/operations.go" {
		ast.Inspect(file, func(node ast.Node) bool {
			assignment, ok := node.(*ast.AssignStmt)
			if !ok {
				return true
			}
			for _, left := range assignment.Lhs {
				if name, ok := left.(*ast.Ident); ok && name.Name == "Operations" {
					out = append(out, finding{rel, 1, "only the descriptor registry may assign Operations"})
				}
			}
			return true
		})
	}
	return out
}
