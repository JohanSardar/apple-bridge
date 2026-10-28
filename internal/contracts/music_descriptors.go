package contracts

func init() {
	RegisterSource("music", "Music", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "music.search_catalog", Version: 1, Kind: QueryOp, Source: "music", Scope: "music:read", Input: Query{}, Output: QueryResult{}, Description: "Search the public iTunes catalog; bounded transient metadata"},
		}
	}, 90)
}
