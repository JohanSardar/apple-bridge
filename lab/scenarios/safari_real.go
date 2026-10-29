package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

func safariReal(ctx context.Context, s Scenario, e Evidence, binary string) Evidence {
	home, _ := os.UserHomeDir()
	directory := filepath.Join(home, "Library", "Safari")
	truth := exec.CommandContext(ctx, "python3", "-c", `
import sqlite3,plistlib,sys,time,hashlib
p=sys.argv[1]
if sys.argv[2]=='SAF-HISTORY-REAL':
 c=sqlite3.connect('file:'+p+'/History.db?mode=ro',uri=True)
 n=c.execute('SELECT COUNT(*) FROM history_visits WHERE visit_time>=?',(time.time()-978307200-7*86400,)).fetchone()[0]
 print(n)
else:
 with open(p+'/Bookmarks.plist','rb') as f:root=plistlib.load(f)
 rows=[];n=0
 def walk(node,folders,reading):
  global n
  if 'Children' in node:
   title=node.get('Title','');folders=folders+([title] if title else [])
   for child in node['Children']:walk(child,folders,reading or title=='com.apple.ReadingList')
  elif node.get('WebBookmarkType')=='WebBookmarkTypeLeaf':
   n+=1
   if not reading:rows.append(node['URLString']+'\0'+node.get('URIDictionary',{}).get('title',node.get('Title',''))+'\0'+'/'.join(folders))
 walk(root,[],False)
 print(n,hashlib.sha256('\n'.join(sorted(rows)).encode()).hexdigest())
`, directory, s.ID)
	expected, err := truth.Output()
	if err != nil {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_truth_access", Status: "limited", Detail: "Truth process cannot read Safari data; Full Disk Access required"})
		return e
	}
	actual, err := exec.CommandContext(ctx, binary, directory, s.ID).Output()
	want := strings.Fields(string(expected))
	got := strings.Fields(string(actual))
	ok := err == nil && len(got) == 2 && len(want) > 0 && got[0] == want[0]
	if s.ID == "SAF-BOOKMARKS-REAL" {
		ok = ok && len(want) == 2 && got[1] == want[1]
	}
	e.Passed = e.Passed && ok
	e.Assertions = append(e.Assertions, Assertion{Name: "independent_truth_counts_and_bookmark_digest", Passed: ok})
	if ok {
		e.Metrics = map[string]any{"count": got[0]}
		if len(want) == 2 {
			e.Metrics["digest"] = got[1]
		}
	}
	return e
}
