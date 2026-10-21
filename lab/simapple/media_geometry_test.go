package simapple

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func geometryBox(kind string, parts ...[]byte) []byte {
	size := 8
	for _, part := range parts {
		size += len(part)
	}
	output := make([]byte, 8, size)
	binary.BigEndian.PutUint32(output, uint32(size))
	copy(output[4:], kind)
	for _, part := range parts {
		output = append(output, part...)
	}
	return output
}
func geometryClock(version byte, scale uint32, duration uint64) []byte {
	offset, size := 12, 20
	if version == 1 {
		offset, size = 20, 32
	}
	data := make([]byte, size)
	data[0] = version
	binary.BigEndian.PutUint32(data[offset:], scale)
	if version == 1 {
		binary.BigEndian.PutUint64(data[offset+4:], duration)
	} else {
		binary.BigEndian.PutUint32(data[offset+4:], uint32(duration))
	}
	return data
}
func geometryMovie(version byte, rotation int, scale uint32, duration uint64) []byte {
	matrix, length := 40, 84
	if version == 1 {
		matrix, length = 52, 96
	}
	header := make([]byte, length)
	header[0] = version
	values := [4]int32{65536, 0, 0, 65536}
	switch rotation {
	case 90:
		values = [4]int32{0, 65536, -65536, 0}
	case -90:
		values = [4]int32{0, -65536, 65536, 0}
	case 180:
		values = [4]int32{-65536, 0, 0, -65536}
	case 1:
		values = [4]int32{0, 0, 0, 0}
	}
	for i, offset := range []int{0, 4, 12, 16} {
		binary.BigEndian.PutUint32(header[matrix+offset:], uint32(values[i]))
	}
	binary.BigEndian.PutUint32(header[matrix+32:], 1<<30)
	binary.BigEndian.PutUint32(header[length-8:], 640<<16)
	binary.BigEndian.PutUint32(header[length-4:], 360<<16)
	handler := make([]byte, 12)
	copy(handler[8:], "vide")
	track := geometryBox("trak", geometryBox("tkhd", header), geometryBox("mdia", geometryBox("mdhd", geometryClock(version, scale, duration)), geometryBox("hdlr", handler)))
	movieTicks := uint64(1200)
	if scale > 0 {
		movieTicks = duration * 600 / uint64(scale)
	}
	return geometryBox("moov", geometryBox("mvhd", geometryClock(version, 600, movieTicks)), track)
}
func geometryExtended(data []byte) []byte {
	header := make([]byte, 16)
	binary.BigEndian.PutUint32(header, 1)
	copy(header[4:], data[4:8])
	binary.BigEndian.PutUint64(header[8:], uint64(len(data)+8))
	return append(header, data[8:]...)
}
func geometryToEnd(data []byte) []byte {
	copy := append([]byte(nil), data...)
	binary.BigEndian.PutUint32(copy, 0)
	return copy
}
func TestSourceVideoGeometry(t *testing.T) {
	tests := []struct {
		name  string
		data  []byte
		want  videoGeometry
		valid bool
	}{
		{"v0", geometryMovie(0, 0, 90000, 225000), videoGeometry{640, 360, 2500}, true},
		{"v1 rotated portrait", geometryMovie(1, 90, 90000, 135000), videoGeometry{360, 640, 1500}, true},
		{"negative rotation", geometryMovie(0, -90, 1000, 1750), videoGeometry{360, 640, 1750}, true},
		{"half turn", geometryMovie(0, 180, 1000, 1250), videoGeometry{640, 360, 1250}, true},
		{"movie clock fallback", geometryMovie(0, 0, 0, 0), videoGeometry{640, 360, 2000}, true},
		{"media clock fallback", bytes.Replace(geometryMovie(0, 0, 1000, 1250), []byte("mvhd"), []byte("free"), 1), videoGeometry{640, 360, 1250}, true},
		{"movie presentation duration", bytes.Replace(geometryMovie(0, 0, 1000, 2500), geometryClock(0, 600, 1500), geometryClock(0, 600, 600), 1), videoGeometry{640, 360, 1000}, true},
		{"landscape fixture", MediaFixture("video"), videoGeometry{320, 180, 1000}, true},
		{"portrait fixture", MediaFixture("video-portrait"), videoGeometry{180, 320, 1000}, true},
		{"ultrawide fixture", MediaFixture("video-panorama"), videoGeometry{1920, 200, 1000}, true},
		{"extended box", geometryExtended(geometryMovie(0, 0, 1000, 1000)), videoGeometry{640, 360, 1000}, true},
		{"box to end", geometryToEnd(geometryMovie(0, 0, 1000, 1000)), videoGeometry{640, 360, 1000}, true},
		{"audio track", bytes.Replace(geometryMovie(0, 0, 1000, 1000), []byte("vide"), []byte("soun"), 1), videoGeometry{}, false},
		{"zero transform", geometryMovie(0, 1, 1000, 1000), videoGeometry{}, false},
		{"unknown version", geometryMovie(2, 0, 1000, 1000), videoGeometry{}, false},
		{"truncated", geometryMovie(0, 0, 1000, 1000)[:60], videoGeometry{}, false},
		{"invalid size", []byte{0, 0, 0, 4, 'm', 'o', 'o', 'v'}, videoGeometry{}, false},
		{"extended short", []byte{0, 0, 0, 1, 'm', 'o', 'o', 'v', 255}, videoGeometry{}, false},
		{"not a movie", []byte("invalid video"), videoGeometry{}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, valid := sourceVideoGeometry(tt.data)
			if valid != tt.valid || got != tt.want {
				t.Fatalf("got %+v %v, want %+v %v", got, valid, tt.want, tt.valid)
			}
		})
	}
}
