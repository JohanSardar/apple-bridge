package contracts

type Grant string
type OpKind string

const ReadOp OpKind = "read"
const CommandOp OpKind = "command"

type Example struct {
	Input  any `json:"input"`
	Output any `json:"output"`
}
type OperationDescriptor struct {
	Name        string
	Version     int
	Kind        OpKind
	Source      Source
	Scope       Grant
	Capability  string
	Input       any
	Output      any
	Description string
	Examples    []Example
}

var Operations = append(CalendarDescriptors(), []OperationDescriptor{
	{Name: "replica.status", Version: 1, Kind: ReadOp, Source: System, Scope: "system:read", Input: struct{}{}, Output: ReadResult{}, Description: "Read local replication and blob progress"},
	{Name: "search", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Search visible local record text"},
	{Name: "records.get", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Read one visible local bridge record"},
	{Name: "records.query", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Page visible local bridge records in collection and ID order"},
	{Name: "sources.status", Version: 1, Kind: ReadOp, Input: struct{}{}, Output: ReadResult{}, Description: "Read source coverage and freshness within scope"},
	{Name: "commands.status", Version: 1, Kind: ReadOp, Input: CommandQuery{}, Output: CommandRecord{}, Description: "Look up an issued command by its original ID; resolve only gathers stronger evidence"},
	{Name: "commands.list", Version: 1, Kind: ReadOp, Input: CommandQuery{}, Output: []CommandRecord{}, Description: "List commands issued by this principal within source scope"},
	{Name: "reminders.create", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a reminder in a writable list"},
	{Name: "reminders.create_list", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.create_list", Input: Command{}, Output: CommandRecord{}, Description: "Create a writable Reminders list"},
	{Name: "reminders.update", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.update", Input: Command{}, Output: CommandRecord{}, Description: "Patch reminder fields with expected_version"},
	{Name: "reminders.complete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.complete", Input: Command{}, Output: CommandRecord{}, Description: "Complete a reminder with expected_version"},
	{Name: "reminders.uncomplete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.uncomplete", Input: Command{}, Output: CommandRecord{}, Description: "Reopen a reminder with expected_version"},
	{Name: "reminders.delete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a reminder with expected_version"},
	{Name: "contacts.lookup", Version: 1, Kind: ReadOp, Source: Contacts, Scope: "contacts:read", Input: ContactLookup{}, Output: ReadResult{}, Description: "Look up unified contacts by name or normalized phone/email"},
	{Name: "contacts.create", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a contact in the default writable container"},
	{Name: "contacts.update", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.update", Input: Command{}, Output: CommandRecord{}, Description: "Patch contact fields with expected_version"},
	{Name: "contacts.delete", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a writable contact with expected_version"},
	{Name: "notes.create", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a plain text note in a folder"},
	{Name: "notes.update", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.update", Input: Command{}, Output: CommandRecord{}, Description: "Replace or append plain text with expected_version; rich content rejects"},
	{Name: "notes.delete", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a note with expected_version"},
	{Name: "notes.create_folder", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.create_folder", Input: Command{}, Output: CommandRecord{}, Description: "Create a Notes folder"},
}...)
