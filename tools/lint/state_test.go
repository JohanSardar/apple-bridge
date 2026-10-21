package main

import "testing"

func TestStateWriteSerialization(t *testing.T) {
	for _, tc := range []struct {
		name string
		body string
		fail bool
	}{
		{"activity", `s.db.BeginTx(ctx,nil)`, true},
		{"returning", `s.db.QueryRowContext(ctx,"UPDATE meta SET value=1 RETURNING value")`, true},
		{"pragma", `s.db.QueryRow("PRAGMA wal_checkpoint(TRUNCATE)")`, true},
		{"prune", `s.db.ExecContext(ctx,"DELETE FROM agent_activity")`, true},
		{"prepare", `s.db.Prepare("INSERT INTO meta VALUES(?,?)")`, true},
		{"read", `s.db.BeginTx(ctx,&sql.TxOptions{ReadOnly:true})`, false},
		{"locked", `s.write.Lock(); defer s.write.Unlock(); s.db.BeginTx(ctx,nil)`, false},
		{"late_lock", `s.db.BeginTx(ctx,nil); s.write.Lock(); defer s.write.Unlock()`, true},
		{"missing_unlock", `s.write.Lock(); s.db.BeginTx(ctx,nil)`, true},
		{"mirror", `s.publishCommandLocked(ctx,r)`, true},
		{"snapshot", `i.store.write.Lock(); defer i.store.write.Unlock(); i.store.db.BeginTx(ctx,nil)`, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			src := []byte("package state; func f(){" + tc.body + "}")
			if got := len(lintStateWrites("internal/state/probe.go", src)) > 0; got != tc.fail {
				t.Fatalf("rejected=%v want %v", got, tc.fail)
			}
		})
	}
	if checkImport("internal/web/probe.go", "database/sql") == "" || checkImport("internal/blobs/protection.go", "database/sql") != "" {
		t.Fatal("SQL module boundary was not enforced")
	}
}
