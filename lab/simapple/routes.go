package simapple

import (
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
)

var sourceRoutes []func(*fakeApple, *http.ServeMux)

type sourceCommand func(*fakeApple, *applepeer.Wire, contracts.IPCFrame, uint64, func(contracts.Capture, uint64) error, func(contracts.Source) string) (bool, error)

var sourceCommands []sourceCommand
