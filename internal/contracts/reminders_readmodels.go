package contracts

type ReminderListsQuery struct {
	TimeZone string `json:"time_zone" description:"Viewer IANA time zone"`
}

type ReminderSmartTile struct {
	ID     string `json:"id" enum:"today,scheduled,all,flagged,completed,assigned"`
	Count  *int   `json:"count,omitempty"`
	Status string `json:"status,omitempty" enum:"unsupported"`
}

type ReminderSection struct {
	Kind   string  `json:"kind" enum:"store,list,date,daypart,none"`
	ID     string  `json:"id"`
	Title  string  `json:"title"`
	Order  float64 `json:"order"`
	ListID string  `json:"list_id,omitempty"`
}

type ReminderList struct {
	List     Record            `json:"list"`
	Count    *int              `json:"count,omitempty"`
	Status   string            `json:"status,omitempty" enum:"unsupported"`
	Sections []ReminderSection `json:"sections"`
	GroupID  *string           `json:"group_id"`
	Order    *float64          `json:"order"`
	Pinned   *bool             `json:"pinned"`
}

type ReminderTagCount struct {
	Name  string `json:"name"`
	Count int    `json:"count"`
}

type ReminderListsResult struct {
	Smart  []ReminderSmartTile `json:"smart"`
	Lists  []ReminderList      `json:"lists"`
	Tags   []ReminderTagCount  `json:"tags"`
	Cursor Cursor              `json:"cursor"`
}

type ReminderItemsQuery struct {
	TimeZone      string `json:"time_zone"`
	List          string `json:"list,omitempty"`
	Smart         string `json:"smart,omitempty" enum:"today,scheduled,all,flagged,completed,assigned"`
	Tag           string `json:"tag,omitempty"`
	ShowCompleted bool   `json:"show_completed,omitempty"`
	Sort          string `json:"sort,omitempty" enum:"manual,due,created,priority,title"`
}

type ReminderItem struct {
	Reminder    Record         `json:"reminder"`
	Subtasks    []ReminderItem `json:"subtasks"`
	Attachments []Record       `json:"attachments"`
}

type ReminderSectionItems struct {
	Section ReminderSection `json:"section"`
	Items   []ReminderItem  `json:"items"`
}

type ReminderItemsResult struct {
	Sections []ReminderSectionItems `json:"sections"`
	Count    int                    `json:"count"`
	Cursor   Cursor                 `json:"cursor"`
}
