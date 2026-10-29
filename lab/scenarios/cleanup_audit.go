package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"time"
)

func cleanupSnapshot(ctx context.Context, source string) ([]cleanupItem, error) {
	data, err := exec.CommandContext(ctx, "python3", "-c", `
import sqlite3,os,json,sys
home=os.path.expanduser('~')
if sys.argv[1]=='messages':
 c=sqlite3.connect('file:'+home+'/Library/Messages/chat.db?mode=ro',uri=True)
 rows=c.execute('SELECT DISTINCT m.guid,c.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=j.chat_id AND r.message_id=m.ROWID)').fetchall()
else:
 c=sqlite3.connect('file:'+home+'/Library/Group Containers/group.com.apple.notes/NoteStore.sqlite?mode=ro',uri=True)
 rows=c.execute("WITH RECURSIVE trash(pk) AS (SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT WHERE ZFOLDERTYPE=1 UNION SELECT f.Z_PK FROM ZICCLOUDSYNCINGOBJECT f JOIN trash t ON f.ZPARENT=t.pk) SELECT ZIDENTIFIER,CAST(COALESCE(ZFOLDER,0) AS TEXT) FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK NOT IN trash AND COALESCE(ZFOLDER,0) NOT IN trash AND Z_ENT IN (SELECT Z_ENT FROM Z_PRIMARYKEY WHERE Z_NAME IN ('ICNote','ICFolder')) AND COALESCE(ZMARKEDFORDELETION,0)=0 AND ZIDENTIFIER IS NOT NULL").fetchall()
print(json.dumps([{'ID':a+'|'+b,'Container':b} for a,b in rows]))
`, source).Output()
	if err != nil {
		return nil, fmt.Errorf("cleanup snapshot unavailable")
	}
	var rows []cleanupItem
	if json.Unmarshal(data, &rows) != nil {
		return nil, fmt.Errorf("invalid cleanup snapshot")
	}
	return rows, nil
}

func finishCleanupAudit(ctx context.Context, c *runCleanup, source string, e *Evidence) {
	c.settle = 30 * time.Second
	proof, err := c.finish(func() ([]cleanupItem, error) { return cleanupSnapshot(ctx, source) }, func(cleanupItem) error { return fmt.Errorf("unregistered cleanup") })
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["cleanup_counts"] = proof
	e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_baseline_restored", Passed: err == nil})
	e.Passed = e.Passed && err == nil
}
