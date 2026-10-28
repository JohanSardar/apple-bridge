package contracts

func MailDescriptors() []OperationDescriptor {
	return []OperationDescriptor{{Name: "mail.body", Version: 1, Kind: QueryOp, Source: Mail, Scope: "mail:read", Input: Query{}, Output: QueryResult{}, Description: "Read local Mail body"}}
}

func init() { registerDescriptors(MailDescriptors, 80) }
