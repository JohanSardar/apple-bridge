package contracts

func init() {
	RegisterSource("print", "Print", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "print.document", Version: 1, Kind: CommandOp, Source: "print", Scope: "print:write", Capability: "print.document", Input: Command{}, Output: CommandRecord{}, Description: "Print a file or authorized blob to PDF or a named printer"},
		}
	})
}
