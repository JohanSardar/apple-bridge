package main

import (
	"bufio"
	"bytes"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	_ "modernc.org/sqlite"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "spike failed:", err)
		os.Exit(1)
	}
}
func run() error {
	if len(os.Args) > 1 && os.Args[1] == "tls" {
		return tlsSpike(os.Args[2:])
	}
	if len(os.Args) > 1 && os.Args[1] == "sqlite" {
		return sqliteSpike(os.Args[2], os.Args[3])
	}
	child := exec.Command(os.Args[1])
	in, e := child.StdinPipe()
	if e != nil {
		return e
	}
	out, e := child.StdoutPipe()
	if e != nil {
		return e
	}
	child.Stderr = os.Stderr
	if e = child.Start(); e != nil {
		return e
	}
	enc := json.NewEncoder(in)
	scan := bufio.NewScanner(out)
	count, mismatches := 0, 0
	check := func(raw []byte, command, nohtml bool) error {
		var want string
		if command {
			var c contracts.Command
			if e := json.Unmarshal(raw, &c); e != nil {
				return e
			}
			h, e := contracts.CommandHash(c)
			if e != nil {
				return e
			}
			want = h
		} else {
			var v any
			d := json.NewDecoder(bytes.NewReader(raw))
			d.UseNumber()
			if e := d.Decode(&v); e != nil {
				return e
			}
			var b bytes.Buffer
			enc := json.NewEncoder(&b)
			enc.SetEscapeHTML(!nohtml)
			if e := enc.Encode(v); e != nil {
				return e
			}
			h := sha256.Sum256(bytes.TrimSuffix(b.Bytes(), []byte{'\n'}))
			want = hex.EncodeToString(h[:])
		}
		request := map[string]any{"raw": string(raw), "command": command, "nohtml": nohtml}
		if !utf8.Valid(raw) {
			octets := make([]int, len(raw))
			for i, b := range raw {
				octets[i] = int(b)
			}
			request["raw_bytes"] = octets
		}
		if e := enc.Encode(request); e != nil {
			return e
		}
		if !scan.Scan() {
			return fmt.Errorf("Rust stopped after %d vectors", count)
		}
		count++
		if scan.Text() != want {
			mismatches++
		}
		return nil
	}
	for _, v := range []string{`null`, `true`, `-0`, `1e400`, `1E-009`, `9007199254740993`, `0.000001`, `1.234567890123456789`, `{"z":"<>&\u2028\u2029\b\f\n\r\t\u0000","a":[-0,1e+09]}`, `{"dup":1,"dup":2}`, `"😀é日本語"`, `"\ud800"`, `"\udfff"`, `"\ud800\udc00"`, `"\\ud800"`, `{"\ud800":1,"\udfff":2}`, `-0.00`, `1e-10000000`} {
		for _, mode := range []bool{false, true} {
			if e = check([]byte(v), false, mode); e != nil {
				return e
			}
		}
	}
	for i := 0; i < 10000; i++ {
		raw := []byte(fmt.Sprintf(`{"n":%d.%09de%+d,"s":"<>&\\\"","a":[null,true]}`, i, i, (i%601)-300))
		if e = check(raw, false, false); e != nil {
			return e
		}
	}
	for _, stamp := range []string{"0001-01-01T00:00:00Z", "2026-11-02T13:14:15.123400000-05:30", "2024-02-29T00:00:00.000000001Z"} {
		raw := []byte(fmt.Sprintf(`{"op":"synthetic.<>&","args":{"n":1E+400,"s":"\u2028"},"expected_version":18446744073709551615,"expires_at":%q,"admission_era":"era"}`, stamp))
		if e = check(raw, true, false); e != nil {
			return e
		}
	}
	for _, raw := range [][]byte{{'"', 0xe2, 0x82, '"'}, {'"', 0xed, 0xa0, 0x80, '"'}, {'{', '"', 0xff, '"', ':', '1', '}'}} {
		if e = check(raw, false, false); e != nil {
			return e
		}
	}
	fmt.Printf("vectors count=%d mismatches=%d\n", count, mismatches)
	for _, path := range os.Args[2:] {
		db, e := sql.Open("sqlite", "file:"+path+"?mode=ro")
		if e != nil {
			return e
		}
		command := strings.HasSuffix(path, "ledger.db")
		query := "SELECT payload FROM records ORDER BY collection,id"
		if command {
			query = "SELECT payload FROM commands ORDER BY id"
		}
		rows, e := db.Query(query)
		if e != nil {
			return e
		}
		start := count
		for rows.Next() {
			var payload []byte
			if e = rows.Scan(&payload); e != nil {
				return e
			}
			var raw json.RawMessage
			if command {
				var r contracts.CommandRecord
				if e = json.Unmarshal(payload, &r); e != nil {
					return e
				}
				raw, e = json.Marshal(r.Command)
			} else {
				var r contracts.Record
				if e = json.Unmarshal(payload, &r); e != nil {
					return e
				}
				raw = r.Data
			}
			if e != nil {
				return e
			}
			if e = check(raw, command, false); e != nil {
				return e
			}
		}
		if e = rows.Err(); e != nil {
			return e
		}
		rows.Close()
		db.Close()
		fmt.Printf("command=%t count=%d cumulative_mismatches=%d\n", command, count-start, mismatches)
	}
	in.Close()
	if e = child.Wait(); e != nil {
		return e
	}
	if mismatches != 0 {
		return fmt.Errorf("%d digest mismatches", mismatches)
	}
	return nil
}
func sqliteSpike(path, binary string) error {
	home := path + "-home"
	store, e := state.Open(home)
	if e != nil {
		return e
	}
	store.Close()
	path = home + "/state.db"
	db, e := sql.Open("sqlite", path)
	if e != nil {
		return e
	}
	var version string
	db.QueryRow("SELECT sqlite_version()").Scan(&version)
	fmt.Println("Go SQLite", version)
	if _, e = db.Exec("PRAGMA journal_mode=WAL; CREATE TABLE spike(n INTEGER); INSERT INTO spike VALUES(1); CREATE VIRTUAL TABLE corpus USING fts5(text, tokenize='unicode61 remove_diacritics 2')"); e != nil {
		return e
	}
	for _, s := range []string{"Hello world café résumé", "hello WORLD", "日本語 alpha", "beta e-mail", "CAFÉ resume"} {
		if _, e = db.Exec("INSERT INTO corpus VALUES(?)", s); e != nil {
			return e
		}
	}
	var expected []string
	for _, q := range []string{"cafe", "hello", "\"hello world\"", "hel*", "日本語", "resume", "alpha OR beta", "NOTHING"} {
		rows, e := db.Query("SELECT rowid FROM corpus WHERE corpus MATCH ? ORDER BY rowid", q)
		if e != nil {
			return e
		}
		ids := []int64{}
		for rows.Next() {
			var id int64
			rows.Scan(&id)
			ids = append(ids, id)
		}
		rows.Close()
		b, _ := json.Marshal(ids)
		expected = append(expected, string(b))
	}
	db.Close()
	result, e := exec.Command(binary, path).Output()
	if e != nil {
		return e
	}
	lines := strings.Split(strings.TrimSpace(string(result)), "\n")
	fmt.Println(lines[0])
	if len(lines) != len(expected)+1 {
		return fmt.Errorf("query count")
	}
	for i, w := range expected {
		if lines[i+1] != w {
			return fmt.Errorf("query mismatch %d", i)
		}
	}
	db, e = sql.Open("sqlite", path)
	if e != nil {
		return e
	}
	defer db.Close()
	var count int
	db.QueryRow("SELECT count(*) FROM corpus").Scan(&count)
	var schema string
	db.QueryRow("SELECT value FROM meta WHERE key='schema_version'").Scan(&schema)
	if count != 6 || schema != "2" {
		return fmt.Errorf("reopen mismatch")
	}
	fmt.Println("queries=8 mismatches=0 snapshot=pass bidirectional=pass schema=2")
	return nil
}
