package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) Invoke(ctx context.Context, p trust.Principal, name string, input json.RawMessage) (output any, resultErr error) {
	d, ok := contracts.Descriptor(name)
	if !ok {
		return nil, contracts.ErrUnsupported
	}
	if p.Kind == "agent" && s.Authority != nil {
		current, err := s.Authority.Principal(p.ID)
		if err != nil {
			return nil, err
		}
		current.Grants = trust.EffectiveGrants(current.Grants, p.Grants)
		p = current
	}
	if d.Kind != contracts.CommandOp && name != "records.get" && name != "records.query" && name != "search" && name != "sources.status" && name != "replica.status" {
		defer func() {
			if err := s.recordAgentRead(context.WithoutCancel(ctx), p, name); err != nil && resultErr == nil {
				resultErr = err
			}
		}()
	}
	switch name {
	case "contacts.list":
		var query contracts.ContactsListQuery
		if contracts.DecodeInput(input, &query) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.ContactsList(ctx, p.Visibility(), query)
	case "calendar.range":
		var query contracts.CalendarRangeQuery
		if contracts.DecodeInput(input, &query) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.CalendarRange(ctx, p.Visibility(), query)
	case "reminders.items":
		var query contracts.ReminderItemsQuery
		if contracts.DecodeInput(input, &query) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.ReminderItems(ctx, p.Visibility(), query)
	case "reminders.lists":
		var query contracts.ReminderListsQuery
		if contracts.DecodeInput(input, &query) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.ReminderLists(ctx, p.Visibility(), query)
	case "messages.handles":
		var q contracts.HandlesQuery
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.Handles(ctx, p.Visibility(), q)
	case "messages.chats":
		var q contracts.ChatsQuery
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.Chats(ctx, p.Visibility(), q)
	case "messages.thread":
		var q contracts.ThreadQuery
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.Thread(ctx, p.Visibility(), q)
	case "messages.search":
		var q contracts.MessageSearchQuery
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.SearchMessages(ctx, p.Visibility(), q)
	case "messages.collection":
		var q contracts.CollectionQuery
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		return s.State.Collection(ctx, p.Visibility(), q)
	}
	if name == "calendar.agenda" {
		return s.agenda(ctx, p, input)
	}
	if d.Kind == contracts.CommandOp {
		var c contracts.Command
		if contracts.DecodeInput(input, &c) != nil {
			return nil, contracts.ErrInvalid
		}
		if c.Op != "" && c.Op != name {
			return nil, contracts.ErrInvalid
		}
		c.Op = name
		if c.ID == "" {
			c.ID = contracts.NewCommandID()
		}
		if s.Target == nil {
			return nil, contracts.ErrUnavailable
		}
		if _, remote := s.Target.(*RemoteGateway); remote && p.Kind == "agent" && p.Level == trust.AgentRead {
			if err := s.State.RecordAgentActivity(context.WithoutCancel(ctx), state.AgentActivity{ID: p.ID + ":" + c.ID, Principal: p.ID, Operation: c.Op, Kind: "command", State: "rejected", At: time.Now().UTC(), ArgumentBytes: len(c.Args), ArgumentHash: trust.Fingerprint(c.Args)}); err != nil {
				return nil, err
			}
			return nil, contracts.ErrUnauthorized
		}
		return s.Target.Submit(ctx, p, c)
	}
	if name == "commands.status" || name == "commands.list" {
		var q contracts.CommandQuery
		if contracts.DecodeInput(input, &q) != nil || q.Limit < 0 || q.Limit > 1000 {
			return nil, contracts.ErrInvalid
		}
		if s.Target == nil {
			return nil, contracts.ErrUnavailable
		}
		if name == "commands.list" {
			return s.Target.List(ctx, p, q)
		}
		if !contracts.ValidCommandID(q.ID) {
			return nil, contracts.ErrInvalid
		}
		return s.Target.Status(ctx, p, q)
	}
	if name == "contacts.lookup" {
		var q contracts.ContactLookup
		if contracts.DecodeInput(input, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		var fields map[string]json.RawMessage
		if json.Unmarshal(input, &fields) != nil || fields["q"] != nil && q.Query == "" {
			return nil, contracts.ErrInvalid
		}
		return s.lookupContacts(ctx, p, q)
	}
	var q contracts.ReadQuery
	if contracts.DecodeInput(input, &q) != nil {
		return nil, contracts.ErrInvalid
	}
	result, err := s.Read(ctx, p, name, q)

	return result, err
}
