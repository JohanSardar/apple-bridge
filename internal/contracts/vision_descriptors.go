package contracts

func init() {
	RegisterSource("vision", "Vision", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "vision.ocr", Version: 1, Kind: QueryOp, Source: "vision", Scope: "vision:read", Input: Query{}, Output: QueryResult{}, Description: "Transient vision.ocr result"},
		}
	}, 68)
}
