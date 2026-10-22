package contracts

type ChatsQuery struct {
	Participants []string `json:"participants,omitempty" description:"Exact normalized phone/email handle set, 1–32 inputs; duplicates folded"`
	Filter       string   `json:"filter,omitempty" description:"all (default), known, unknown, unread, deleted, spam, or archived"`
	Limit        int      `json:"limit,omitempty" description:"Page size, default 50, maximum 200"`
	After        string   `json:"after,omitempty" description:"Opaque cursor from the previous page"`
}
type ThreadQuery struct {
	ThreadOriginatorID string `json:"thread_originator_id,omitempty" description:"Restrict to this root message and its replies in the chat"`
	ChatID             string `json:"chat_id"`
	Anchor             string `json:"anchor,omitempty" description:"latest (default), oldest, unread, or a message ID"`
	Before             string `json:"before,omitempty"`
	After              string `json:"after,omitempty"`
	Limit              int    `json:"limit,omitempty" description:"Page size, default 50, maximum 200"`
}
type CollectionQuery struct {
	ChatID string `json:"chat_id"`
	Kind   string `json:"kind" enum:"photos,videos,media,links,documents,locations,audio"`
	After  string `json:"after,omitempty"`
	Limit  int    `json:"limit,omitempty"`
}
type AvatarHandle struct {
	ID        string  `json:"id"`
	ContactID *string `json:"contact_id"`
}
type MessagePreview struct {
	GroupEvent      map[string]any   `json:"group_event,omitempty"`
	Kind            string           `json:"kind" enum:"text,attachments,reaction,group_event,unsent,edited"`
	Text            string           `json:"text,omitempty"`
	AttachmentKinds []string         `json:"attachment_kinds,omitempty"`
	Count           int              `json:"count,omitempty"`
	Reaction        *ReactionPreview `json:"reaction,omitempty"`
}
type ReactionPreview struct {
	Kind          string `json:"kind,omitempty"`
	Removed       bool   `json:"removed,omitempty"`
	Verb          string `json:"verb"`
	TargetExcerpt string `json:"target_excerpt"`
}
type LastMessage struct {
	SenderHandleID *string        `json:"sender_handle_id,omitempty"`
	ID             string         `json:"id"`
	SentAt         string         `json:"sent_at"`
	IsFromMe       bool           `json:"is_from_me"`
	Preview        MessagePreview `json:"preview"`
}
type PinPreview struct {
	MessageID   string `json:"message_id"`
	TextPreview string `json:"text_preview"`
	At          string `json:"at"`
}
type ChatSummary struct {
	KnownSender      bool           `json:"known_sender"`
	PinPreview       *PinPreview    `json:"pin_preview"`
	Chat             Record         `json:"chat"`
	Title            string         `json:"title"`
	AvatarHandles    []AvatarHandle `json:"avatar_handles"`
	LastMessage      *LastMessage   `json:"last_message"`
	LastActivityAt   string         `json:"last_activity_at"`
	UnreadCount      int            `json:"unread_count"`
	HasUnseenMention bool           `json:"has_unseen_mention"`
	Muted            *bool          `json:"muted"`
	PinnedIndex      *int           `json:"pinned_index"`
}
type ChatsResult struct {
	Chats         []ChatSummary `json:"chats"`
	Next          string        `json:"next,omitempty"`
	Cursor        Cursor        `json:"cursor"`
	CoverageNotes []string      `json:"coverage_notes"`
}
type ThreadItem struct {
	Record
	Reactions   []Record `json:"reactions"`
	Attachments []Record `json:"attachments"`
	ReplyCount  int      `json:"reply_count"`
}
type ThreadResult struct {
	Items        []ThreadItem `json:"items"`
	Before       string       `json:"before,omitempty"`
	After        string       `json:"after,omitempty"`
	UnreadAnchor string       `json:"unread_anchor,omitempty"`
	Cursor       Cursor       `json:"cursor"`
}
type CollectionResult struct {
	Items         []Record `json:"items"`
	Next          string   `json:"next,omitempty"`
	Cursor        Cursor   `json:"cursor"`
	CoverageNotes []string `json:"coverage_notes"`
}

type HandlesQuery struct {
	IDs []string `json:"ids" description:"At most 500 handle IDs"`
}
type HandleAvatar struct {
	Digest  string `json:"digest"`
	Variant string `json:"variant"`
}
type HandleSummary struct {
	HandleID    string        `json:"handle_id"`
	Address     string        `json:"address"`
	Service     string        `json:"service"`
	ContactID   *string       `json:"contact_id,omitempty"`
	DisplayName string        `json:"display_name"`
	GivenName   string        `json:"given_name,omitempty"`
	FamilyName  string        `json:"family_name,omitempty"`
	Initials    string        `json:"initials"`
	Avatar      *HandleAvatar `json:"avatar,omitempty"`
}
type HandlesResult struct {
	Handles []HandleSummary `json:"handles"`
	Cursor  Cursor          `json:"cursor"`
}
