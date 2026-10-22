//go:build !lab

package web

func commandObserveFailure(stage string) error { return nil }

func commandStatusLookup(id string)        {}
func commandVisibilityHeld(id string) bool { return false }

func commandStatusReset() {}
