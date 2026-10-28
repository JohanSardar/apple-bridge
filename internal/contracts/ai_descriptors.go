package contracts

func init() {
	RegisterSource("ai", "On-device AI", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "ai.local", Version: 1, Kind: QueryOp, Source: "ai", Scope: "ai:read", Input: Query{}, Output: QueryResult{}, Description: "Apple on-device model response or availability reason"},
			{Name: "ai.rewrite", Version: 1, Kind: QueryOp, Source: "ai", Scope: "ai:read", Input: Query{}, Output: QueryResult{}, Description: "Apple on-device rewrite or availability reason"},
		}
	}, 74, 75)
}
