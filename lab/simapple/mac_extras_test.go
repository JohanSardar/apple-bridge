package simapple

import (
	"bytes"
	"encoding/base64"
	"testing"
)

func TestIWorkExportPreservesDocument(t *testing.T) {
	for _, text := range []string{"[ab-test]", "[ab-test] (second)\\document"} {
		data, err := base64.StdEncoding.DecodeString(fakeIWorkExport("text", text))
		if err != nil || string(data) != text {
			t.Fatal("text export did not preserve the supplied document")
		}
	}
	pdf, err := base64.StdEncoding.DecodeString(fakeIWorkExport("pdf", "[ab-test] (second)\\document"))
	if err != nil || !bytes.HasPrefix(pdf, []byte("%PDF-")) || !bytes.Contains(pdf, []byte(`[ab-test] \(second\)\\document`)) {
		t.Fatal("PDF export did not escape the supplied document")
	}
}
