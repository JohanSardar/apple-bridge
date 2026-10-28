package contracts

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"strings"
)

type PhotoCommandArgs struct {
	Data     string `json:"data,omitempty"`
	Digest   string `json:"digest,omitempty"`
	Kind     string `json:"kind,omitempty"`
	Album    string `json:"album,omitempty"`
	Asset    string `json:"asset,omitempty"`
	Name     string `json:"name,omitempty"`
	Favorite *bool  `json:"favorite,omitempty"`
}

func ValidatePhotoCommand(c Command) error {
	var a PhotoCommandArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(c.Args, &fields) != nil {
		return ErrInvalid
	}
	allowed := ""
	switch c.Op {
	case "photos.import":
		allowed = " data digest kind album "
		bytes, err := base64.StdEncoding.DecodeString(a.Data)
		sum := sha256.Sum256(bytes)
		if err != nil || len(bytes) == 0 || len(bytes) > 16<<20 || hex.EncodeToString(sum[:]) != a.Digest || (a.Kind != "image" && a.Kind != "video") {
			return ErrInvalid
		}
	case "photos.create_album":
		allowed = " name "
		if strings.TrimSpace(a.Name) == "" || len(a.Name) > 1024 {
			return ErrInvalid
		}
	case "photos.set_favorite":
		allowed = " asset favorite "
		if a.Asset == "" || a.Favorite == nil {
			return ErrInvalid
		}
	case "photos.delete":
		allowed = " asset "
		if a.Asset == "" {
			return ErrInvalid
		}
	case "photos.add_to_album", "photos.remove_from_album":
		allowed = " asset album "
		if a.Asset == "" || a.Album == "" {
			return ErrInvalid
		}
	default:
		return ErrInvalid
	}
	for key := range fields {
		if !strings.Contains(allowed, " "+key+" ") {
			return ErrInvalid
		}
	}
	return nil
}
