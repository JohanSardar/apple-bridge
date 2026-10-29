package contracts

func init() {
	RegisterSource("speech", "Speech", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "speech.synthesize", Version: 1, Kind: QueryOp, Source: "speech", Scope: "speech:read", Input: Query{}, Output: QueryResult{}, Description: "Transient synthesized WAV audio"},
		}
	}, 72)
}
