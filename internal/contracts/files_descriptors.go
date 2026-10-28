package contracts

func init() {
	registerDescriptors(FileCommandDescriptors, 70, 71, 72, 73, 74, 75, 76, 77)
	RegisterSource("files", "Files", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "files.search", Version: 1, Kind: QueryOp, Source: "files", Scope: "files:read", Input: Query{}, Output: QueryResult{}, Description: "Search opted-in file roots"},
			{Name: "files.read", Version: 1, Kind: QueryOp, Source: "files", Scope: "files:read", Input: Query{}, Output: QueryResult{}, Description: "Read an opted-in file"},
		}
	}, 68, 69)
	messagesShape["files.item"] = map[string]fieldShape{"root": requiredField(shapeString), "path": requiredField(shapeString), "name": requiredField(shapeString), "kind": requiredField(shapeString), "size": requiredField(shapeNumber), "created": nullableField(shapeString), "modified": nullableField(shapeString), "tags": requiredField(shapeArray), "downloaded": requiredField(shapeBool), "icloud_status": requiredField(shapeString)}
}

func FileCommandDescriptors() []OperationDescriptor {
	var out []OperationDescriptor
	for _, op := range []string{"files.create", "files.write", "files.move", "files.rename", "files.trash", "files.set_tags", "files.download", "files.evict"} {
		out = append(out, OperationDescriptor{Name: op, Version: 1, Kind: CommandOp, Source: "files", Scope: "files:write", Capability: op, Input: Command{}, Output: CommandRecord{}, Description: "Mutate an opted-in file with native readback"})
	}
	return out
}
