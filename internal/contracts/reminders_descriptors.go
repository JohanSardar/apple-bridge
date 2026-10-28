package contracts

func RemindersDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{
		{Name: "reminders.items", Version: 1, Kind: ReadOp, ReadModel: true, Source: Reminders, Scope: "reminders:read", Input: ReminderItemsQuery{}, Output: ReminderItemsResult{}, Description: "Read visible reminders with sections, subtasks and attachments in the viewer time zone"},
		{Name: "reminders.lists", Version: 1, Kind: ReadOp, ReadModel: true, Source: Reminders, Scope: "reminders:read", Input: ReminderListsQuery{}, Output: ReminderListsResult{}, Description: "Read native list and group order, tags and smart counts in the viewer time zone; unsupported counts are omitted"},
		{Name: "reminders.create", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a reminder in a writable list"},
		{Name: "reminders.create_list", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.create_list", Input: Command{}, Output: CommandRecord{}, Description: "Create a writable Reminders list"},
		{Name: "reminders.update", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.update", Input: Command{}, Output: CommandRecord{}, Description: "Patch reminder fields with expected_version"},
		{Name: "reminders.complete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.complete", Input: Command{}, Output: CommandRecord{}, Description: "Complete a reminder with expected_version"},
		{Name: "reminders.uncomplete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.uncomplete", Input: Command{}, Output: CommandRecord{}, Description: "Reopen a reminder with expected_version"},
		{Name: "reminders.delete", Version: 1, Kind: CommandOp, Source: Reminders, Scope: "reminders:write", Capability: "reminders.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a reminder with expected_version"},
	}
	return descriptors
}

func init() { registerDescriptors(RemindersDescriptors, 8, 9, 22, 23, 24, 25, 26, 27) }
