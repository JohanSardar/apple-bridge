//go:build !lab

package scenarios

import (
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
)

func webUploadCheckpointControl(http.ResponseWriter, *http.Request, *world.World) bool { return false }

func uploadControlLabEnabled() bool { return false }
