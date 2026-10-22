package contracts

type ContactsListQuery struct {
	Group        string `json:"group,omitempty"`
	Sort         string `json:"sort,omitempty" enum:"given,family"`
	DisplayOrder string `json:"display_order,omitempty" enum:"given,family"`
}
type ContactListItem struct {
	Contact         Record        `json:"contact"`
	DisplayName     string        `json:"display_name"`
	SortKey         string        `json:"sort_key"`
	Initials        string        `json:"initials"`
	Avatar          *HandleAvatar `json:"avatar"`
	IsMe            *bool         `json:"is_me"`
	ConversationIDs []string      `json:"conversation_ids"`
}
type ContactListSection struct {
	Key      string            `json:"key"`
	Contacts []ContactListItem `json:"contacts"`
}
type ContactListGroup struct {
	Group Record `json:"group"`
	Count int    `json:"count"`
}
type ContactsListResult struct {
	Sections     []ContactListSection `json:"sections"`
	Groups       []ContactListGroup   `json:"groups"`
	Containers   []Record             `json:"containers"`
	Count        int                  `json:"count"`
	Sort         string               `json:"sort"`
	DisplayOrder string               `json:"display_order"`
	Cursor       Cursor               `json:"cursor"`
}
