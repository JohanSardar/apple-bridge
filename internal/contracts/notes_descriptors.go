package contracts

func NotesDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{
		{Name: "notes.list", Version: 1, Kind: ReadOp, ReadModel: true, Source: Notes, Scope: "notes:read", Input: NotesListQuery{}, Output: NotesListResult{}, Description: "Read Notes accounts, nested folders, pinned and date-grouped notes with previews and thumbnail references; locked notes are title-only and Recently Deleted is read-only"},
		{Name: "notes.create", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a plain text note in a folder"},
		{Name: "notes.update", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.update", Input: Command{}, Output: CommandRecord{}, Description: "Replace or append plain text with expected_version; rich content rejects"},
		{Name: "notes.delete", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a note with expected_version"},
		{Name: "notes.create_folder", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.create_folder", Input: Command{}, Output: CommandRecord{}, Description: "Create a Notes folder"},
	}
	descriptors = append(descriptors, OperationDescriptor{Name: "notes.move", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.move", Input: Command{}, Output: CommandRecord{}, Description: "Move a note to a writable folder with expected_version"})
	descriptors = append(descriptors, OperationDescriptor{Name: "notes.rename_folder", Version: 1, Kind: CommandOp, Source: Notes, Scope: "notes:write", Capability: "notes.rename_folder", Input: Command{}, Output: CommandRecord{}, Description: "Rename a tagged Bridge Test folder with expected_version"})
	return descriptors
}

func init() { registerDescriptors(NotesDescriptors, 6, 37, 38, 39, 40, 71, 72) }
