package contracts

func ContactsDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{
		{Name: "contacts.list", Version: 1, Kind: ReadOp, ReadModel: true, Source: Contacts, Scope: "contacts:read", Input: ContactsListQuery{}, Output: ContactsListResult{}, Description: "Read grouped contacts in deterministic A–Z sections with viewer name preferences, identity, avatar references and scope-filtered linked conversations"},
		{Name: "contacts.lookup", Version: 1, Kind: ReadOp, Source: Contacts, Scope: "contacts:read", Input: ContactLookup{}, Output: ReadResult{}, Description: "Look up unified contacts by partial q (name, phone or email), capped at 25 in deterministic name/ID order; legacy name and exact handle filters remain supported"},
		{Name: "contacts.create", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.create", Input: Command{}, Output: CommandRecord{}, Description: "Create a contact in the default writable container"},
		{Name: "contacts.update", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.update", Input: Command{}, Output: CommandRecord{}, Description: "Patch contact fields with expected_version"},
		{Name: "contacts.delete", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.delete", Input: Command{}, Output: CommandRecord{}, Description: "Delete a writable contact with expected_version"},
		{Name: "contacts.create_group", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.create_group", Input: Command{}, Output: CommandRecord{}, Description: "Create a contact group in the default container"},
		{Name: "contacts.rename_group", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.rename_group", Input: Command{}, Output: CommandRecord{}, Description: "Rename a writable contact group with expected_version"},
		{Name: "contacts.delete_group", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.delete_group", Input: Command{}, Output: CommandRecord{}, Description: "Delete a writable contact group without deleting its contacts"},
		{Name: "contacts.add_member", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.add_member", Input: Command{}, Output: CommandRecord{}, Description: "Add a contact from the same container to a group"},
		{Name: "contacts.remove_member", Version: 1, Kind: CommandOp, Source: Contacts, Scope: "contacts:write", Capability: "contacts.remove_member", Input: Command{}, Output: CommandRecord{}, Description: "Remove group membership without deleting the contact"},
	}
	return descriptors
}

func init() { registerDescriptors(ContactsDescriptors, 7, 28, 29, 30, 31, 32, 33, 34, 35, 36) }
