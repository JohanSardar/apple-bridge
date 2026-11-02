package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
	"sort"
)

func main() {
	s, err := state.Open(os.Args[1])
	if err != nil {
		panic(err)
	}
	defer s.Close()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 16<<20)
	for scanner.Scan() {
		var q struct {
			Op         string                `json:"op"`
			Visibility trust.Visibility      `json:"visibility"`
			Query      json.RawMessage       `json:"query"`
			Operation  string                `json:"operation"`
			Batch      contracts.Batch       `json:"batch"`
			Begin      contracts.FeedFrame   `json:"begin"`
			Pages      []contracts.FeedFrame `json:"pages"`
			Epoch      string                `json:"epoch"`
			Revision   uint64                `json:"revision"`
			Cursor     contracts.Cursor      `json:"cursor"`
		}
		if err = json.Unmarshal(scanner.Bytes(), &q); err != nil {
			panic(err)
		}
		var out any
		ctx := context.Background()
		err = nil
		switch q.Op {
		case "references":
			refs, e := s.BlobReferenceSet(ctx)
			err = e
			owners := []string{}
			for owner := range refs {
				owners = append(owners, owner)
			}
			sort.Strings(owners)
			outrefs := []contracts.BlobReference{}
			for _, owner := range owners {
				list := refs[owner]
				sort.Slice(list, func(i, j int) bool { return list[i].Digest < list[j].Digest })
				outrefs = append(outrefs, list...)
			}
			out = outrefs
		case "restrict":
			err = s.Restrict(ctx, q.Visibility)
			out = map[string]any{}
		case "head":
			out = s.Head()
		case "changes":
			var i *state.ChangeIterator
			i, err = s.FeedChanges(ctx, q.Visibility, q.Cursor)
			if err == nil {
				out, err = i.Next(ctx)
			}
		case "read":
			var query contracts.ReadQuery
			json.Unmarshal(q.Query, &query)
			out, err = s.Read(ctx, q.Visibility, query)
		case "apply":
			out, err = s.ApplyBatch(ctx, q.Batch)
		case "discard":
			err = s.DiscardCapture(ctx)
			out = map[string]any{}
		case "snapshot":
			var i *state.SnapshotInstall
			i, err = s.BeginInstall(ctx, q.Begin, q.Epoch, q.Revision)
			if err == nil {
				for _, p := range q.Pages {
					err = i.Page(ctx, p)
					if err != nil {
						break
					}
				}
			}
			if err == nil {
				out, err = i.InstallSnapshot(ctx)
			}
		case "digest":
			var r contracts.ReadResult
			r, err = s.Read(ctx, q.Visibility, contracts.ReadQuery{Limit: 10000})
			out = state.Digest(r.Records)
		case "model":
			switch q.Operation {
			case "contacts.lookup", "notes.list", "health.summary", "calendar.range", "replica.status":
				service := operations.Service{State: s}
				out, err = service.Invoke(ctx, trust.Principal{ID: "synthetic", Kind: "owner", Grants: []contracts.Grant{"admin"}}, q.Operation, q.Query)
			case "contacts.list":
				var query contracts.ContactsListQuery
				json.Unmarshal(q.Query, &query)
				out, err = s.ContactsList(ctx, q.Visibility, query)
			case "sources.status":
				out, err = s.SourceStatuses(ctx, q.Visibility)
			case "calendar.agenda":
				service := operations.Service{State: s}
				out, err = service.Invoke(ctx, trust.Principal{ID: "synthetic", Kind: "owner", Grants: []contracts.Grant{"calendar:read"}}, q.Operation, q.Query)
			case "messages.chats":
				var x contracts.ChatsQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.Chats(ctx, q.Visibility, x)
			case "messages.thread":
				var x contracts.ThreadQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.Thread(ctx, q.Visibility, x)
			case "messages.search":
				var x contracts.MessageSearchQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.SearchMessages(ctx, q.Visibility, x)
			case "messages.collection":
				var x contracts.CollectionQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.Collection(ctx, q.Visibility, x)
			case "messages.handles":
				var x contracts.HandlesQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.Handles(ctx, q.Visibility, x)
			case "reminders.lists":
				var x contracts.ReminderListsQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.ReminderLists(ctx, q.Visibility, x)
			case "reminders.items":
				var x contracts.ReminderItemsQuery
				json.Unmarshal(q.Query, &x)
				out, err = s.ReminderItems(ctx, q.Visibility, x)
			}
		}
		if err != nil {
			out = map[string]any{"error": err.Error()}
		}
		raw, _ := json.Marshal(out)
		var canonical any
		json.Unmarshal(raw, &canonical)
		raw, _ = json.Marshal(canonical)
		fmt.Println(string(raw))
	}
}
