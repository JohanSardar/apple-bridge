package scenarios

import "testing"

func TestPhotosReportPassed(t *testing.T) {
	for _, test := range []struct {
		name, report string
		want         bool
	}{
		{"complete", `{"grant_passed":true,"read_passed":true,"import_passed":true,"original_passed":true,"cleanup_passed":true}`, true},
		{"cleanup_failed", `{"grant_passed":true,"read_passed":true,"import_passed":true,"original_passed":true,"cleanup_passed":false}`, false},
		{"failure", `{"grant_passed":true,"read_passed":true,"import_passed":true,"original_passed":true,"cleanup_passed":true,"failure":"cleanup failed"}`, false},
		{"grant_denied", `{"grant_passed":false}`, false},
		{"missing", `{}`, false},
		{"invalid", `{`, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			if got := photosReportPassed([]byte(test.report)); got != test.want {
				t.Fatalf("got %t, want %t", got, test.want)
			}
		})
	}
}
