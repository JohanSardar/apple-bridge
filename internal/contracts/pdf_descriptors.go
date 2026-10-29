package contracts

func init() {
	RegisterSource("pdf", "PDF", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "pdf.annotate", Version: 1, Kind: CommandOp, Source: "pdf", Scope: "pdf:write", Capability: "pdf.annotate", Input: Command{}, Output: CommandRecord{}, Description: "Create an annotated PDF artifact"},
			{Name: "pdf.text", Version: 1, Kind: QueryOp, Source: "pdf", Scope: "pdf:read", Input: Query{}, Output: QueryResult{}, Description: "Transient pdf.text result"},
			{Name: "pdf.search", Version: 1, Kind: QueryOp, Source: "pdf", Scope: "pdf:read", Input: Query{}, Output: QueryResult{}, Description: "Transient pdf.search result"},
		}
	}, 97, 66, 67)
	Collections["pdf.artifact"] = "pda"
	messagesShape["pdf.artifact"] = map[string]fieldShape{"blob": requiredField(shapeObject), "annotation_count": requiredField(shapeNumber)}
}
