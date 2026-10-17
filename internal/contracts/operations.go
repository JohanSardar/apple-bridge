package contracts

type Grant string
type OpKind string

const ReadOp OpKind = "read"
const CommandOp OpKind = "command"

type Example struct {
	Input  any `json:"input"`
	Output any `json:"output"`
}
type OperationDescriptor struct {
	Name        string
	Version     int
	Kind        OpKind
	Source      Source
	Scope       Grant
	Capability  string
	Input       any
	Output      any
	Description string
	Examples    []Example
}

var Operations = []OperationDescriptor{
	{Name: "replica.status", Version: 1, Kind: ReadOp, Source: System, Scope: "system:read", Input: struct{}{}, Output: ReadResult{}, Description: "Read local replication and blob progress"},
	{Name: "records.get", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Read one visible local bridge record"},
	{Name: "records.query", Version: 1, Kind: ReadOp, Input: ReadQuery{}, Output: ReadResult{}, Description: "Page visible local bridge records in collection and ID order"},
	{Name: "sources.status", Version: 1, Kind: ReadOp, Input: struct{}{}, Output: ReadResult{}, Description: "Read source coverage and freshness within scope"},
}
