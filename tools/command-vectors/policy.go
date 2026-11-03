package main

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"time"
)

func policies() {
	rows := []map[string]any{}
	now := time.Date(2026, 11, 2, 12, 0, 0, 0, time.UTC)
	for _, op := range []string{"messages.send", "messages.edit", "reminders.create", "iphone.ping"} {
		ttl, max := contracts.CommandTTL(op)
		for _, delta := range []time.Duration{-1, 0, 1, ttl, max, max + 1} {
			end := now.Add(delta)
			rows = append(rows, map[string]any{"kind": "expiry", "op": op, "now": now, "requested": end, "accepted": end.After(now) && end.Sub(now) <= max})
		}
		rows = append(rows, map[string]any{"kind": "expiry", "op": op, "now": now, "requested": nil, "accepted": true, "expires": now.Add(ttl)})
	}
	for _, delta := range [][4]int64{{0, 1, 2, 4}, {0, -2, -1, 4}, {0, 0, 0, 0}, {0, 1, 0, 3}, {3, 1, 2, 0}, {0, 0, 9, 1}, {0, -3, -3, 1}} {
		t := [4]time.Time{}
		for i, d := range delta {
			t[i] = now.Add(time.Duration(d))
		}
		offset, uncertainty, err := (contracts.ClockSample{T0: t[0], T1: t[1], T2: t[2], T3: t[3]}).Estimate()
		rows = append(rows, map[string]any{"kind": "clock", "times": t, "accepted": err == nil, "offset": offset.Nanoseconds(), "uncertainty": uncertainty.Nanoseconds()})
	}
	for _, original := range []string{"<p>a</p>", "<p>a<b>b</b></p>", "<table>x</table>", "<p data-notes-read-only>x</p>", "<p>x<"} {
		for _, replacement := range []string{"<p>b</p>", "<p>x<b>y</b></p>", "<div>x</div>", "<p></p>", "<p>x<", "<p>\x00</p>"} {
			data, _ := json.Marshal(map[string]any{"html": original, "content_status": "complete", "block_status": "complete", "round_trip_status": "unproven", "attachments": []any{}})
			rows = append(rows, map[string]any{"kind": "html", "data": json.RawMessage(data), "html": replacement, "accepted": contracts.NoteHTMLCompatible(data, replacement)})
		}
	}
	b, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		panic(err)
	}
	fmt.Println(string(b))
}
