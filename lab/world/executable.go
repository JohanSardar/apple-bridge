package world

import (
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
)

type executableImage struct {
	file   *os.File
	name   string
	digest string
	size   int64
	err    error
}

var labImage = captureExecutable()

func captureExecutable() executableImage {
	image := executableImage{}
	image.name, image.err = os.Executable()
	if image.err != nil {
		return image
	}
	image.file, image.err = os.Open(image.name)
	if image.err != nil {
		return image
	}
	info, err := image.file.Stat()
	if err != nil {
		image.file.Close()
		image.err = err
		return image
	}
	image.size = info.Size()
	hash := sha256.New()
	_, image.err = io.Copy(hash, io.NewSectionReader(image.file, 0, image.size))
	if image.err != nil {
		image.file.Close()
		return image
	}
	image.digest = hex.EncodeToString(hash.Sum(nil))
	return image
}

func stageExecutable(destination string) error {
	if labImage.err != nil {
		return labImage.err
	}
	if os.Link(labImage.name, destination) == nil {
		original, first := labImage.file.Stat()
		linked, second := os.Stat(destination)
		if first == nil && second == nil && os.SameFile(original, linked) {
			return nil
		}
		if err := os.Remove(destination); err != nil {
			return err
		}
	}
	output, err := os.OpenFile(destination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0700)
	if err != nil {
		return err
	}
	_, copyErr := io.Copy(output, io.NewSectionReader(labImage.file, 0, labImage.size))
	closeErr := output.Close()
	if copyErr != nil {
		os.Remove(destination)
		return copyErr
	}
	if closeErr != nil {
		os.Remove(destination)
		return closeErr
	}
	return nil
}
