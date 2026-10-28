package relay

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func testHello(host string) []byte {
	b := make([]byte, 34)
	b[0] = 3
	b[1] = 3
	b = append(b, 0, 0, 2, 0x13, 1, 1, 0)
	ext := []byte{0, 0, 0, 0, 0, 0, 0, 0, 0}
	binary.BigEndian.PutUint16(ext[2:4], uint16(len(host)+5))
	binary.BigEndian.PutUint16(ext[4:6], uint16(len(host)+3))
	binary.BigEndian.PutUint16(ext[7:9], uint16(len(host)))
	ext = append(ext, host...)
	b = binary.BigEndian.AppendUint16(b, uint16(len(ext)))
	b = append(b, ext...)
	n := len(b)
	return append([]byte{1, byte(n >> 16), byte(n >> 8), byte(n)}, b...)
}
func TestFragmentedClientHello(t *testing.T) {
	hello := testHello("device.relay.test")
	for split := 1; split < len(hello); split++ {
		raw := []byte{}
		for _, b := range [][]byte{hello[:split], hello[split:]} {
			raw = append(raw, 22, 3, 1)
			raw = binary.BigEndian.AppendUint16(raw, uint16(len(b)))
			raw = append(raw, b...)
		}
		host, forward, err := clientHello(bytes.NewReader(raw))
		if err != nil || host != "device.relay.test" || !bytes.Equal(forward, raw) {
			t.Fatalf("fragment %d rejected or changed", split)
		}
	}
}
func TestMalformedClientHello(t *testing.T) {
	for _, input := range [][]byte{nil, {23, 3, 3, 0, 1, 0}, {22, 3, 3, 255, 255}, testHello("invalid_host.relay.test")} {
		if _, _, err := clientHello(bytes.NewReader(input)); err == nil {
			t.Fatal("malformed input accepted")
		}
	}
	for n := 0; n < len(testHello("a.relay.test"))-4; n++ {
		if _, err := helloName(testHello("a.relay.test")[4:][:n]); err == nil {
			t.Fatal("truncated hello accepted")
		}
	}
}
func FuzzHelloName(f *testing.F) {
	f.Add(testHello("device.relay.test")[4:])
	f.Add([]byte{})
	f.Fuzz(func(t *testing.T, b []byte) {
		host, err := helloName(b)
		if err == nil && !validHost(host) {
			t.Fatal("invalid hostname")
		}
	})
}
