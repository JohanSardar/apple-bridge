package simapple

import (
	"os"
	"path/filepath"
	"syscall"
)

func messageUploadBoundary(path string) bool {
	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		return false
	}
	parent, err := os.Stat(filepath.Dir(resolved))
	if err != nil || filepath.Base(filepath.Dir(resolved)) != "message-uploads" || !parent.IsDir() || parent.Mode()&(os.ModePerm|os.ModeSticky|os.ModeSetuid|os.ModeSetgid) != 0700 {
		return false
	}
	file, err := os.Stat(resolved)
	if err != nil || !file.Mode().IsRegular() {
		return false
	}
	parentOwner, parentOK := parent.Sys().(*syscall.Stat_t)
	fileOwner, fileOK := file.Sys().(*syscall.Stat_t)
	return parentOK && fileOK && parentOwner.Uid == uint32(os.Getuid()) && fileOwner.Uid == uint32(os.Getuid())
}
