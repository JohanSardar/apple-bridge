package contracts

import "sort"

type Grant string
type OpKind string

const QueryOp OpKind = "query"
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
	ReadModel   bool
	Source      Source
	Scope       Grant
	Capability  string
	Input       any
	Output      any
	Description string
	Examples    []Example
}

type descriptorRegistration struct {
	source    func() []OperationDescriptor
	positions []int
}

var descriptorRegistry []descriptorRegistration
var Operations []OperationDescriptor

func registerDescriptors(source func() []OperationDescriptor, positions ...int) {
	descriptorRegistry = append(descriptorRegistry, descriptorRegistration{source, positions})
	Operations = collectDescriptors()
}

func collectDescriptors() []OperationDescriptor {
	type orderedDescriptor struct {
		descriptor OperationDescriptor
		position   int
	}
	var ordered []orderedDescriptor
	for _, registration := range descriptorRegistry {
		for i, descriptor := range registration.source() {
			position := int(^uint(0) >> 1)
			if i < len(registration.positions) {
				position = registration.positions[i]
			}
			ordered = append(ordered, orderedDescriptor{descriptor, position})
		}
	}
	sort.Slice(ordered, func(i, j int) bool {
		if ordered[i].position != ordered[j].position {
			return ordered[i].position < ordered[j].position
		}
		return ordered[i].descriptor.Name < ordered[j].descriptor.Name
	})
	var descriptors []OperationDescriptor
	for _, entry := range ordered {
		descriptors = append(descriptors, entry.descriptor)
	}
	return descriptors
}
