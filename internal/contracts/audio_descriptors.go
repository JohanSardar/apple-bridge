package contracts

func init() {
	RegisterSource("audio", "Audio", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "audio.transcribe", Version: 1, Kind: QueryOp, Source: "audio", Scope: "audio:read", Input: Query{}, Output: QueryResult{}, Description: "On-device transcription without authorization prompts"},
		}
	}, 73)
}
