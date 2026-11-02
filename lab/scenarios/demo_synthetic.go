package scenarios

import (
	"bytes"
	"regexp"
	"strings"
)

func demoSynthetic(raw []byte, blobs map[string][]byte) bool {
	email := regexp.MustCompile(`[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}`)
	phone := regexp.MustCompile(`\+[0-9][0-9 -]{6,}[0-9]`)
	check := func(data []byte) bool {
		for _, value := range email.FindAll(data, -1) {
			if !strings.HasSuffix(string(value), "@example.com") {
				return false
			}
		}
		for _, value := range phone.FindAll(data, -1) {
			if !regexp.MustCompile(`^\+1 555 01[0-9]{2}$`).Match(value) {
				return false
			}
		}
		return true
	}
	if !check(raw) {
		return false
	}
	for _, blob := range blobs {
		if bytes.HasPrefix(blob, []byte("BEGIN:VCARD")) || bytes.HasPrefix(blob, []byte("%PDF")) {
			if !check(blob) {
				return false
			}
		}
	}
	return true
}
