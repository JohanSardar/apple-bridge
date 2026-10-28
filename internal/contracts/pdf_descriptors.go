package contracts

func init() {
	RegisterSource("pdf", "PDF", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "pdf.text", Version: 1, Kind: QueryOp, Source: "pdf", Scope: "pdf:read", Input: Query{}, Output: QueryResult{}, Description: "Transient pdf.text result"},
			{Name: "pdf.search", Version: 1, Kind: QueryOp, Source: "pdf", Scope: "pdf:read", Input: Query{}, Output: QueryResult{}, Description: "Transient pdf.search result"},
		}
	}, 66, 67)
}
