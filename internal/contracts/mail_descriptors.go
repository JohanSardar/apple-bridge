package contracts

func MailDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{{Name: "mail.body", Version: 1, Kind: QueryOp, Source: Mail, Scope: "mail:read", Input: Query{}, Output: QueryResult{}, Description: "Read local Mail body"}}
	for _, op := range []string{"mail.create_draft", "mail.send", "mail.move", "mail.set_flag", "mail.mark_read", "mail.delete"} {
		descriptors = append(descriptors, OperationDescriptor{Name: op, Version: 1, Kind: CommandOp, Source: Mail, Scope: "mail:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Mail effect with readback evidence"})
	}
	return descriptors
}

func init() { registerDescriptors(MailDescriptors, 80, 90, 91, 92, 93, 94, 95) }
