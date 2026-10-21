package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"strconv"
	"strings"
)

func selectorPath(e ast.Expr) string {
	switch v := e.(type) {
	case *ast.Ident:
		return v.Name
	case *ast.SelectorExpr:
		return selectorPath(v.X) + "." + v.Sel.Name
	}
	return ""
}
func readOnlyTransaction(e ast.Expr) bool {
	pointer, ok := e.(*ast.UnaryExpr)
	if !ok {
		return false
	}
	options, ok := pointer.X.(*ast.CompositeLit)
	if !ok {
		return false
	}
	for _, field := range options.Elts {
		if pair, ok := field.(*ast.KeyValueExpr); ok && selectorPath(pair.Key) == "ReadOnly" && selectorPath(pair.Value) == "true" {
			return true
		}
	}
	return false
}
func lintStateWrites(rel string, src []byte) []finding {
	if !strings.HasPrefix(filepath.ToSlash(rel), "internal/state/") {
		return nil
	}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, rel, src, 0)
	if err != nil {
		return []finding{{rel, 0, err.Error()}}
	}
	var out []finding
	for _, declaration := range file.Decls {
		fn, ok := declaration.(*ast.FuncDecl)
		if !ok || fn.Body == nil || strings.HasSuffix(fn.Name.Name, "Locked") {
			continue
		}
		locks := map[string]token.Pos{}
		unlocks := map[string]bool{}
		ast.Inspect(fn.Body, func(n ast.Node) bool {
			if call, ok := n.(*ast.CallExpr); ok {
				path := selectorPath(call.Fun)
				if strings.HasSuffix(path, ".write.Lock") {
					locks[strings.TrimSuffix(path, ".Lock")] = call.Pos()
				}
			}
			if deferred, ok := n.(*ast.DeferStmt); ok {
				path := selectorPath(deferred.Call.Fun)
				if strings.HasSuffix(path, ".write.Unlock") {
					unlocks[strings.TrimSuffix(path, ".Unlock")] = true
				}
			}
			return true
		})
		ast.Inspect(fn.Body, func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}
			method, ok := call.Fun.(*ast.SelectorExpr)
			if !ok {
				return true
			}
			owner := selectorPath(method.X)
			write := strings.HasSuffix(method.Sel.Name, "Locked")
			if strings.HasSuffix(owner, ".db") {
				owner = strings.TrimSuffix(owner, ".db")
				write = strings.HasPrefix(method.Sel.Name, "Exec") || strings.HasPrefix(method.Sel.Name, "Prepare") || method.Sel.Name == "Begin" || method.Sel.Name == "BeginTx" && (len(call.Args) < 2 || !readOnlyTransaction(call.Args[1]))
				for _, argument := range call.Args {
					if literal, ok := argument.(*ast.BasicLit); ok && literal.Kind == token.STRING {
						query, _ := strconv.Unquote(literal.Value)
						for _, verb := range []string{"UPDATE ", "INSERT ", "DELETE ", "REPLACE ", "CREATE ", "ALTER ", "DROP ", "VACUUM", "PRAGMA ", "WITH "} {
							write = write || strings.HasPrefix(strings.ToUpper(strings.TrimSpace(query)), verb)
						}
						break
					}
				}
			}
			mutex := owner + ".write"
			if write && (locks[mutex] == token.NoPos || locks[mutex] > call.Pos() || !unlocks[mutex]) {
				out = append(out, finding{rel, fset.Position(call.Pos()).Line, "State database writes require Store.write.Lock and deferred Unlock"})
			}
			return true
		})
	}
	return out
}
