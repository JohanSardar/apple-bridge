package contracts

import (
	"encoding/binary"
	"encoding/hex"
)

const BlobChunkSize = 1 << 20

func EncodeBlobChunk(d string, offset int64, b []byte) ([]byte, error) {
	if !ValidDigest(d) || offset < 0 || len(b) > BlobChunkSize {
		return nil, ErrInvalid
	}
	raw, _ := hex.DecodeString(d)
	out := make([]byte, 64+len(b))
	copy(out, raw)
	binary.BigEndian.PutUint64(out[32:40], uint64(offset))
	binary.BigEndian.PutUint32(out[40:44], uint32(len(b)))
	copy(out[64:], b)
	return out, nil
}
func DecodeBlobChunk(b []byte) (string, int64, []byte, error) {
	if len(b) < 64 || len(b) > 64+BlobChunkSize || int(binary.BigEndian.Uint32(b[40:44])) != len(b)-64 {
		return "", 0, nil, ErrCorrupt
	}
	offset := binary.BigEndian.Uint64(b[32:40])
	if offset > 1<<63-1 {
		return "", 0, nil, ErrInvalid
	}
	for _, v := range b[44:64] {
		if v != 0 {
			return "", 0, nil, ErrInvalid
		}
	}
	return hex.EncodeToString(b[:32]), int64(offset), b[64:], nil
}
