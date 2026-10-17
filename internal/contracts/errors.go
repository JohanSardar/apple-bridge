package contracts

import "errors"

type Error struct {
	Code      string `json:"code"`
	Message   string `json:"message"`
	Retryable bool   `json:"retryable"`
}

func (e *Error) Error() string { return e.Code + ": " + e.Message }
func (e *Error) Is(target error) bool {
	var other *Error
	return errors.As(target, &other) && other.Code == e.Code
}

var (
	ErrUnsupported   = &Error{Code: "unsupported", Message: "operation unsupported"}
	ErrUnauthorized  = &Error{Code: "unauthorized", Message: "credential or scope rejected"}
	ErrUnavailable   = &Error{Code: "unavailable", Message: "peer unavailable", Retryable: true}
	ErrConflict      = &Error{Code: "conflict", Message: "state conflict"}
	ErrReset         = &Error{Code: "reset", Message: "history must be reset"}
	ErrExpired       = &Error{Code: "expired", Message: "credential expired"}
	ErrCorrupt       = &Error{Code: "corrupt", Message: "invalid integrity evidence"}
	ErrCapacity      = &Error{Code: "capacity", Message: "capacity exceeded", Retryable: true}
	ErrIndeterminate = &Error{Code: "indeterminate", Message: "effect cannot be established"}
	ErrInvalid       = &Error{Code: "invalid", Message: "invalid input"}
	ErrNotFound      = &Error{Code: "not_found", Message: "record not found"}
)
