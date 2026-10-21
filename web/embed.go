package webui

import (
	"embed"
	"io/fs"
)

//go:embed all:dist
var files embed.FS

var Assets fs.FS = files
