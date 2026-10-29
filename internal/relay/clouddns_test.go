package relay

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestChallengeKeepsDeviceAddressExplicit(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("name") == "*.relay.test." {
			fmt.Fprint(w, `{"rrsets":[{"name":"*.relay.test.","type":"A","ttl":60,"rrdatas":["192.0.2.1","192.0.2.2"]}]}`)
			return
		}
		fmt.Fprint(w, `{"rrsets":[]}`)
	}))
	defer server.Close()
	dns := &CloudDNS{Client: server.Client(), Suffix: "relay.test"}
	record, err := dns.hostRecord(context.Background(), server.URL, "device.relay.test.")
	if err != nil {
		t.Fatal(err)
	}
	if record == nil || record.Name != "device.relay.test." || record.Type != "A" || len(record.Data) != 2 {
		t.Fatalf("missing explicit device address: %+v", record)
	}
}
