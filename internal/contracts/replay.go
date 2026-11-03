//go:build !lab

package contracts

import "time"

func DecisionNow() time.Time { return time.Now() }

func ReplayRecordID(source Source, generation, collection, native string) (string, bool) {
	return "", false
}

func ReplayCommandID() (string, bool) { return "", false }
