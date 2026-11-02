package main

import (
	"os"
	"regexp"
	"strings"
)

func lintRust(rel, path string) []finding {
	b, err := os.ReadFile(path)
	if err != nil {
		return []finding{{rel, 0, err.Error()}}
	}
	s := string(b)
	var out []finding
	line := 1
	for i := 0; i < len(s); {
		if s[i] == '\n' {
			line++
			i++
			continue
		}
		if s[i] == 'r' || s[i] == 'b' || s[i] == 'c' {
			start := i
			if s[i] != 'r' {
				i++
			}
			if i < len(s) && s[i] == 'r' {
				j := i + 1
				for j < len(s) && s[j] == '#' {
					j++
				}
				if j < len(s) && s[j] == '"' {
					end := "\"" + strings.Repeat("#", j-i-1)
					k := strings.Index(s[j+1:], end)
					if k < 0 {
						break
					}
					next := j + 1 + k + len(end)
					line += strings.Count(s[start:next], "\n")
					i = next
					continue
				}
			}
			i = start
		}
		if s[i] == '"' || s[i] == '\'' && i+2 < len(s) && (s[i+2] == '\'' || s[i+1] == '\\') {
			q := s[i]
			i++
			for i < len(s) {
				if s[i] == '\n' {
					line++
				}
				if s[i] == '\\' {
					i += 2
					continue
				}
				if s[i] == q {
					i++
					break
				}
				i++
			}
			continue
		}
		if strings.HasPrefix(s[i:], "//") {
			end := strings.IndexByte(s[i:], '\n')
			if end < 0 {
				end = len(s) - i
			}
			if !(strings.HasPrefix(rel, "rust/crates/ab-sys/") && strings.HasPrefix(s[i:i+end], "// SAFETY:")) {
				out = append(out, finding{rel, line, "comment"})
			}
			i += end
			continue
		}
		if strings.HasPrefix(s[i:], "/*") {
			out = append(out, finding{rel, line, "comment"})
			i += 2
			continue
		}
		if s[i] == '#' && rustDocAttribute.MatchString(s[i:]) {
			out = append(out, finding{rel, line, "doc attribute"})
		}
		i++
	}
	return out
}

func lintCargo(rel, path string) []finding {
	src, err := os.ReadFile(path)
	if err != nil {
		return []finding{{rel, 0, err.Error()}}
	}
	var out []finding
	for _, feature := range []string{"arbitrary_precision", "preserve_order"} {
		if strings.Contains(string(src), "\""+feature+"\"") || strings.Contains(string(src), "'"+feature+"'") {
			out = append(out, finding{rel, 0, "banned JSON feature: " + feature})
		}
	}
	return out
}

var rustDocAttribute = regexp.MustCompile(`^#\s*!?\s*\[\s*doc\s*(?:=|\()`)
