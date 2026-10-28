package contracts

func init() {
	RegisterSource("files", "Files", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "files.search", Version: 1, Kind: QueryOp, Source: "files", Scope: "files:read", Input: Query{}, Output: QueryResult{}, Description: "Search opted-in file roots"},
			{Name: "files.read", Version: 1, Kind: QueryOp, Source: "files", Scope: "files:read", Input: Query{}, Output: QueryResult{}, Description: "Read an opted-in file"},
		}
	}, 68, 69)
	messagesShape["files.item"] = map[string]fieldShape{"root": requiredField(shapeString), "path": requiredField(shapeString), "name": requiredField(shapeString), "kind": requiredField(shapeString), "size": requiredField(shapeNumber), "created": nullableField(shapeString), "modified": nullableField(shapeString), "tags": requiredField(shapeArray), "downloaded": requiredField(shapeBool), "icloud_status": requiredField(shapeString)}
}
