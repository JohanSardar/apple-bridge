package contracts

func init() {
	RegisterSource("photos", "Photos", true)
	registerDescriptors(func() []OperationDescriptor {
		out := []OperationDescriptor{}
		for _, name := range []string{"photos.search", "photos.original", "photos.thumbnail"} {
			out = append(out, OperationDescriptor{Name: name, Version: 1, Kind: QueryOp, Source: "photos", Scope: "photos:read", Input: Query{}, Output: QueryResult{}, Description: "Query the authorized Photos library; OCR disabled"})
		}
		return out
	}, 81, 82, 83)
	messagesShape["photos.asset"] = map[string]fieldShape{"local_id": requiredField(shapeString), "created": nullableField(shapeString), "kind": requiredField(shapeString), "size": nullableField(shapeNumber), "duration": requiredField(shapeNumber), "favorite": requiredField(shapeBool), "hidden": requiredField(shapeBool), "location": nullableField(shapeObject), "albums": requiredField(shapeArray), "cloud_state": requiredField(shapeString)}
	messagesShape["photos.album"] = map[string]fieldShape{"name": requiredField(shapeString), "kind": requiredField(shapeString), "count": requiredField(shapeNumber)}
}

func init() {
	registerDescriptors(func() []OperationDescriptor {
		out := []OperationDescriptor{}
		for _, name := range []string{"photos.import", "photos.create_album", "photos.add_to_album", "photos.remove_from_album", "photos.set_favorite", "photos.delete"} {
			out = append(out, OperationDescriptor{Name: name, Version: 1, Kind: CommandOp, Source: "photos", Scope: "photos:write", Capability: name, Input: Command{}, Output: CommandRecord{}, Description: "Change the authorized Photos library with native readback"})
		}
		return out
	}, 84, 85, 86, 87, 88, 89)
}
