package relay

import (
	"encoding/binary"
	"errors"
	"io"
	"strings"
)

func validHost(host string) bool {
	if len(host) > 253 || !strings.Contains(host, ".") {
		return false
	}
	for _, label := range strings.Split(host, ".") {
		if len(label) == 0 || len(label) > 63 || label[0] == '-' || label[len(label)-1] == '-' {
			return false
		}
		for _, c := range label {
			if !(c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-') {
				return false
			}
		}
	}
	return true
}
func clientHello(r io.Reader) (string, []byte, error) {
	raw := []byte{}
	hello := []byte{}
	for len(raw) < 65536 {
		header := make([]byte, 5)
		if _, err := io.ReadFull(r, header); err != nil {
			return "", nil, err
		}
		n := int(binary.BigEndian.Uint16(header[3:]))
		if header[0] != 22 || header[1] != 3 || n == 0 || n > 16384 || len(raw)+5+n > 65536 {
			break
		}
		body := make([]byte, n)
		if _, err := io.ReadFull(r, body); err != nil {
			return "", nil, err
		}
		raw = append(raw, header...)
		raw = append(raw, body...)
		hello = append(hello, body...)
		if len(hello) < 4 {
			continue
		}
		size := int(hello[1])<<16 | int(hello[2])<<8 | int(hello[3])
		if hello[0] != 1 || size > 65532 {
			break
		}
		if len(hello) < size+4 {
			continue
		}
		host, err := helloName(hello[4 : 4+size])
		return host, raw, err
	}
	return "", nil, errors.New("invalid ClientHello")
}
func helloName(b []byte) (string, error) {
	bad := errors.New("invalid SNI")
	take := func(n int) []byte {
		if n < 0 || n > len(b) {
			b = nil
			return nil
		}
		out := b[:n]
		b = b[n:]
		return out
	}
	if take(34) == nil || len(b) < 1 {
		return "", bad
	}
	n := int(b[0])
	take(1)
	if take(n) == nil || len(b) < 2 {
		return "", bad
	}
	n = int(binary.BigEndian.Uint16(b))
	take(2)
	if n == 0 || n%2 != 0 || take(n) == nil || len(b) < 1 {
		return "", bad
	}
	n = int(b[0])
	take(1)
	if n == 0 || take(n) == nil || len(b) < 2 {
		return "", bad
	}
	n = int(binary.BigEndian.Uint16(b))
	take(2)
	if n != len(b) {
		return "", bad
	}
	host := ""
	for len(b) >= 4 {
		kind := binary.BigEndian.Uint16(b)
		n = int(binary.BigEndian.Uint16(b[2:]))
		take(4)
		ext := take(n)
		if ext == nil {
			return "", bad
		}
		if kind != 0 {
			continue
		}
		if host != "" || len(ext) < 2 || int(binary.BigEndian.Uint16(ext)) != len(ext)-2 {
			return "", bad
		}
		ext = ext[2:]
		for len(ext) >= 3 {
			kind := ext[0]
			n = int(binary.BigEndian.Uint16(ext[1:]))
			ext = ext[3:]
			if n > len(ext) {
				return "", bad
			}
			if kind == 0 {
				if host != "" {
					return "", bad
				}
				host = strings.ToLower(string(ext[:n]))
			}
			ext = ext[n:]
		}
		if len(ext) != 0 {
			return "", bad
		}
	}
	if len(b) != 0 || !validHost(host) {
		return "", bad
	}
	return host, nil
}
