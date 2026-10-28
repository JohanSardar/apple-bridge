package contracts

func MessagesDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{
		{Name: "messages.chats", Version: 1, Kind: ReadOp, ReadModel: true, Source: Messages, Scope: "messages:read", Input: ChatsQuery{}, Output: ChatsResult{}, Description: "Page visible local chat summaries by newest activity, optionally matching an exact normalized participant handle set through indexes; sender filters, unread and unseen mentions; mute and pinned state remain nullable", Examples: []Example{{Input: ChatsQuery{Limit: 50}, Output: ChatsResult{Chats: []ChatSummary{}, CoverageNotes: []string{}}}}},
		{Name: "messages.thread", Version: 1, Kind: ReadOp, ReadModel: true, Source: Messages, Scope: "messages:read", Input: ThreadQuery{}, Output: ThreadResult{}, Description: "Read a local chat in sent_at and ID order with current reactions, complete attachments and reply counts; opaque before/after cursors, latest, oldest, unread or message anchors, and an optional thread_originator_id filter for a root and all its replies", Examples: []Example{{Input: ThreadQuery{ChatID: "cht_example", Anchor: "unread", Limit: 100}, Output: ThreadResult{Items: []ThreadItem{}}}}},
		{Name: "messages.handles", Version: 1, Kind: ReadOp, ReadModel: true, Source: Messages, Scope: "messages:read", Input: HandlesQuery{}, Output: HandlesResult{}, Description: "Resolve up to 500 visible handles with contact names, initials and avatars"},
		{Name: "messages.search", Version: 1, Kind: ReadOp, ReadModel: true, Source: Messages, Scope: "messages:read", Input: MessageSearchQuery{}, Output: MessageSearchResult{}, Description: "Search visible local messages and attachment filenames with prefix text, sender, media-kind and unread filters; filters apply before pagination"},
		{Name: "messages.collection", Version: 1, Kind: ReadOp, ReadModel: true, Source: Messages, Scope: "messages:read", Input: CollectionQuery{}, Output: CollectionResult{}, Description: "Page visible chat attachments or balloon messages by media kind", Examples: []Example{{Input: CollectionQuery{ChatID: "cht_example", Kind: "photos", Limit: 50}, Output: CollectionResult{Items: []Record{}, CoverageNotes: []string{}}}}},
	}
	for _, op := range append(append([]string{}, MessagesOperations...), "messages.keep_audio") {
		descriptors = append(descriptors, OperationDescriptor{Name: op, Version: 1, Kind: CommandOp, Source: Messages, Scope: "messages:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Messages command with evidence; unproven native routes reject before dispatch"})
	}
	return descriptors
}

func init() {
	registerDescriptors(MessagesDescriptors, 10, 11, 12, 13, 14, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61)
}
