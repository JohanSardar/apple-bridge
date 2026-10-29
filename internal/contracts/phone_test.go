package contracts

import (
	"encoding/json"
	"testing"
)

func TestPhoneDialContract(t *testing.T) {
	d, ok := Descriptor("phone.dial")
	if !ok || d.Scope != "phone:write" || d.Capability != "phone.dial" {
		t.Fatal("dial descriptor")
	}
	for _, number := range []string{"+15555550122", "5550122", "tel:123", "123;456", "123?x=1", "", "+", "123\n"} {
		raw, _ := json.Marshal(PhoneDialArgs{Number: number})
		err := ValidatePhoneDial(Command{ID: "fixture", Op: "phone.dial", Args: raw})
		valid := number == "+15555550122" || number == "5550122"
		if (err == nil) != valid {
			t.Fatal("number validation")
		}
	}
}
