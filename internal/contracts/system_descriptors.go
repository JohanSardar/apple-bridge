package contracts

func SystemDescriptors() []OperationDescriptor {
	descriptors := []OperationDescriptor{
		{Name: "replica.status", Version: 1, Kind: ReadOp, Source: System, Scope: "system:read", Input: struct{}{}, Output: ReadResult{}, Description: "Read local replication and blob progress"},
		{Name: "search", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Search visible local record text"},
		{Name: "records.get", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Read one visible local bridge record"},
		{Name: "records.query", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Page visible local bridge records in collection and ID order"},
		{Name: "sources.status", Version: 1, Kind: ReadOp, Input: struct{}{}, Output: ReadResult{}, Description: "Read source coverage and freshness within scope"},
		{Name: "commands.status", Version: 1, Kind: ReadOp, Input: CommandQuery{}, Output: CommandRecord{}, Description: "Look up an issued command by its original ID; resolve only gathers stronger evidence"},
		{Name: "commands.list", Version: 1, Kind: ReadOp, Input: CommandQuery{}, Output: []CommandRecord{}, Description: "List commands issued by this principal within source scope"},
	}
	return descriptors
}

func init() { registerDescriptors(SystemDescriptors, 15, 16, 17, 18, 19, 20, 21) }
