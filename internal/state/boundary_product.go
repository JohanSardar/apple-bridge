//go:build !lab

package state

func initializeApplyBoundary(s *Store) {}

func captureDiscarded(string, bool) {}
