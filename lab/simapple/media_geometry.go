package simapple

import (
	"encoding/binary"
	"math"
)

type videoGeometry struct{ width, height, durationMS int }

func movieBoxes(data []byte, visit func(string, []byte)) bool {
	for len(data) > 0 {
		if len(data) < 8 {
			return false
		}
		size, header := uint64(binary.BigEndian.Uint32(data)), 8
		if size == 1 {
			if len(data) < 16 {
				return false
			}
			size, header = binary.BigEndian.Uint64(data[8:]), 16
		}
		if size == 0 {
			size = uint64(len(data))
		}
		if size < uint64(header) || size > uint64(len(data)) {
			return false
		}
		visit(string(data[4:8]), data[header:int(size)])
		data = data[int(size):]
	}
	return true
}

func movieDuration(data []byte) int {
	if len(data) < 20 {
		return 0
	}
	offset := 12
	if data[0] == 1 {
		offset = 20
		if len(data) < 32 {
			return 0
		}
	} else if data[0] != 0 {
		return 0
	}
	scale := uint64(binary.BigEndian.Uint32(data[offset:]))
	ticks := uint64(binary.BigEndian.Uint32(data[offset+4:]))
	if data[0] == 1 {
		ticks = binary.BigEndian.Uint64(data[offset+4:])
	} else if ticks == 1<<32-1 {
		return 0
	}
	limit := uint64(^uint(0) >> 1)
	if scale == 0 || ticks == ^uint64(0) || ticks/scale > limit/1000 {
		return 0
	}
	value := ticks/scale*1000 + ticks%scale*1000/scale
	if value > limit {
		return 0
	}
	return int(value)
}

func movieDimensions(data []byte) (int, int) {
	if len(data) < 84 {
		return 0, 0
	}
	matrix, end := 40, 84
	if data[0] == 1 {
		matrix, end = 52, 96
		if len(data) < end {
			return 0, 0
		}
	} else if data[0] != 0 {
		return 0, 0
	}
	width, height := float64(binary.BigEndian.Uint32(data[end-8:]))/65536, float64(binary.BigEndian.Uint32(data[end-4:]))/65536
	values := [4]float64{}
	for i, offset := range []int{0, 4, 12, 16} {
		values[i] = float64(int32(binary.BigEndian.Uint32(data[matrix+offset:]))) / 65536
	}
	x, y := math.Round(math.Abs(values[0])*width+math.Abs(values[2])*height), math.Round(math.Abs(values[1])*width+math.Abs(values[3])*height)
	if x < 1 || y < 1 || x > 1<<20 || y > 1<<20 {
		return 0, 0
	}
	return int(x), int(y)
}

func sourceVideoGeometry(data []byte) (videoGeometry, bool) {
	var movie []byte
	if !movieBoxes(data, func(kind string, body []byte) {
		if kind == "moov" {
			movie = body
		}
	}) || len(movie) == 0 {
		return videoGeometry{}, false
	}
	var clock []byte
	tracks := [][]byte{}
	if !movieBoxes(movie, func(kind string, body []byte) {
		if kind == "mvhd" {
			clock = body
		}
		if kind == "trak" && len(tracks) < 128 {
			tracks = append(tracks, body)
		}
	}) {
		return videoGeometry{}, false
	}
	selected := videoGeometry{}
	for _, track := range tracks {
		var header, media []byte
		if !movieBoxes(track, func(kind string, body []byte) {
			if kind == "tkhd" {
				header = body
			}
			if kind == "mdia" {
				media = body
			}
		}) {
			continue
		}
		var handler, timing []byte
		if !movieBoxes(media, func(kind string, body []byte) {
			if kind == "hdlr" {
				handler = body
			}
			if kind == "mdhd" {
				timing = body
			}
		}) || len(handler) < 12 || string(handler[8:12]) != "vide" {
			continue
		}
		width, height := movieDimensions(header)
		duration := movieDuration(clock)
		if duration == 0 {
			duration = movieDuration(timing)
		}
		if width > 0 && height > 0 && duration > 0 && int64(width)*int64(height) > int64(selected.width)*int64(selected.height) {
			selected = videoGeometry{width, height, duration}
		}
	}
	return selected, selected.width > 0
}
