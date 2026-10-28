package contracts

import (
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

func TestLegacyDescriptorOrder(t *testing.T) {
	data, err := os.ReadFile("testdata/legacy-descriptor-names.json")
	if err != nil {
		t.Fatal(err)
	}
	var want []string
	if err := json.Unmarshal(data, &want); err != nil {
		t.Fatal(err)
	}
	var got []string
	for _, d := range Operations {
		got = append(got, d.Name)
	}
	if len(got) < len(want) || !reflect.DeepEqual(got[:len(want)], want) {
		t.Fatalf("descriptor sequence = %v, want %v", got, want)
	}
}

func TestDescriptorRegistrationOrderAndAppend(t *testing.T) {
	original := descriptorRegistry
	descriptorRegistry = append([]descriptorRegistration(nil), original...)
	defer func() { descriptorRegistry = original }()
	for i, j := 0, len(descriptorRegistry)-1; i < j; i, j = i+1, j-1 {
		descriptorRegistry[i], descriptorRegistry[j] = descriptorRegistry[j], descriptorRegistry[i]
	}
	got := collectDescriptors()
	if !reflect.DeepEqual(got, Operations) {
		t.Fatal("registration order changed descriptors")
	}
	descriptorRegistry = append(descriptorRegistry, descriptorRegistration{source: func() []OperationDescriptor { return []OperationDescriptor{{Name: "aaa.new"}} }})
	got = collectDescriptors()
	if !reflect.DeepEqual(got[:len(Operations)], Operations) || got[len(Operations)].Name != "aaa.new" {
		t.Fatal("new source did not append")
	}
}
