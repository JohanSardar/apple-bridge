package world

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

func fixtureFileDigest(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err = io.Copy(hash, file); err != nil {
		return "", err
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}

func fixtureFingerprint(inputs, gateway, simulator string) (string, error) {
	gatewayHash, err := fixtureFileDigest(gateway)
	if err != nil {
		return "", err
	}
	simulatorHash, err := fixtureFileDigest(simulator)
	if err != nil {
		return "", err
	}
	inputHash := sha256.Sum256([]byte(inputs))
	raw, _ := json.Marshal([]string{"fixture-v1", runtime.Version(), hex.EncodeToString(inputHash[:]), gatewayHash, simulatorHash})
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:]), nil
}

func copyFixtureTree(source, destination string, filter bool) (string, error) {
	if err := os.MkdirAll(destination, 0700); err != nil {
		return "", err
	}
	if !filter && runtime.GOOS == "darwin" {
		if exec.Command("/bin/cp", "-cR", source+"/.", destination).Run() == nil {
			return "clonefile", nil
		}
		if err := os.RemoveAll(destination); err != nil {
			return "", err
		}
		if err := os.MkdirAll(destination, 0700); err != nil {
			return "", err
		}
	}
	method := "clonefile"
	if runtime.GOOS != "darwin" {
		method = "copy"
	}
	err := filepath.WalkDir(source, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		relative, err := filepath.Rel(source, path)
		if err != nil {
			return err
		}
		if relative == "." {
			return nil
		}
		if filter && (entry.Name() == "run" || entry.Name() == "apps" || strings.HasSuffix(entry.Name(), ".log") || entry.Name() == "pid" || entry.Name() == "runtime.json") {
			if entry.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}
		target := filepath.Join(destination, relative)
		if entry.IsDir() {
			return os.MkdirAll(target, 0700)
		}
		if !entry.Type().IsRegular() {
			return nil
		}
		if runtime.GOOS == "darwin" && exec.Command("/bin/cp", "-c", path, target).Run() == nil {
			return os.Chmod(target, 0600)
		}
		method = "copy"
		os.Remove(target)
		in, err := os.Open(path)
		if err != nil {
			return err
		}
		defer in.Close()
		out, err := os.OpenFile(target, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if err != nil {
			return err
		}
		_, copyErr := io.Copy(out, in)
		closeErr := out.Close()
		if copyErr != nil {
			return copyErr
		}
		return closeErr
	})
	return method, err
}
