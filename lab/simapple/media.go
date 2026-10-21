package simapple

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"image"
	"image/color"
	_ "image/gif"
	"image/jpeg"
	"image/png"
	"math"
	"net/http"
	"path/filepath"
	"strings"
)

func (f *fakeApple) mediaVariants(name string, original []byte) (map[string]any, map[string]any) {
	derivatives := map[string]any{}
	media := map[string]any{}
	save := func(name, mime string, data []byte, width, height int) {
		digest := fmt.Sprintf("%x", sha256.Sum256(data))
		f.blobs[digest] = data
		variant := map[string]any{"digest": digest, "state": "present", "size": len(data), "mime": mime}
		if width > 0 {
			variant["width"] = width
			variant["height"] = height
		}
		derivatives[name] = variant
	}
	input, format, err := image.Decode(bytes.NewReader(original))
	if err == nil {
		bounds := input.Bounds()
		media["width"] = bounds.Dx()
		media["height"] = bounds.Dy()
		small := mediaResize(input, 100)
		media["thumbhash"] = mediaThumbHash(small)
		for _, variant := range []struct {
			name             string
			maximum, quality int
		}{{"thumb", 400, 80}, {"display", 1600, 85}} {
			if format == "gif" && variant.name == "display" {
				continue
			}
			output := mediaResize(input, variant.maximum)
			var buffer bytes.Buffer
			mime := "image/jpeg"
			if output.Opaque() {
				jpeg.Encode(&buffer, output, &jpeg.Options{Quality: variant.quality})
			} else {
				png.Encode(&buffer, output)
				mime = "image/png"
			}
			save(variant.name, mime, buffer.Bytes(), output.Bounds().Dx(), output.Bounds().Dy())
		}
	} else {
		ext := strings.ToLower(filepath.Ext(name))
		switch ext {
		case ".mov", ".mp4", ".3gp":
			clip, _ := base64.StdEncoding.DecodeString(simVideo)
			save("video", "video/mp4", clip, 16, 16)
			poster := image.NewNRGBA(image.Rect(0, 0, 16, 16))
			for y := 0; y < 16; y++ {
				for x := 0; x < 16; x++ {
					poster.SetNRGBA(x, y, color.NRGBA{R: 40, G: 120, B: 200, A: 255})
				}
			}
			var buffer bytes.Buffer
			jpeg.Encode(&buffer, poster, &jpeg.Options{Quality: 80})
			save("poster", "image/jpeg", buffer.Bytes(), 16, 16)
			media["width"] = 16
			media["height"] = 16
			media["duration_ms"] = 1000
			media["thumbhash"] = mediaThumbHash(poster)
		case ".caf", ".m4a", ".mp3", ".amr", ".wav":
			clip, _ := base64.StdEncoding.DecodeString(simAudio)
			save("audio", "audio/mp4", clip, 0, 0)
			peaks := make([]int, 64)
			media["waveform"] = peaks
			media["duration_ms"] = 1000
		}
	}
	return derivatives, media
}

func mediaResize(input image.Image, maximum int) *image.NRGBA {
	b := input.Bounds()
	scale := math.Min(1, float64(maximum)/float64(max(b.Dx(), b.Dy())))
	w, h := max(1, int(float64(b.Dx())*scale)), max(1, int(float64(b.Dy())*scale))
	output := image.NewNRGBA(image.Rect(0, 0, w, h))
	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			output.Set(x, y, input.At(b.Min.X+x*b.Dx()/w, b.Min.Y+y*b.Dy()/h))
		}
	}
	return output
}

func mediaThumbHash(input *image.NRGBA) string {
	w, h := input.Bounds().Dx(), input.Bounds().Dy()
	count := w * h
	average := [4]float64{}
	for i := 0; i < count; i++ {
		a := float64(input.Pix[i*4+3]) / 255
		for c := 0; c < 3; c++ {
			average[c] += a / 255 * float64(input.Pix[i*4+c])
		}
		average[3] += a
	}
	if average[3] > 0 {
		for c := 0; c < 3; c++ {
			average[c] /= average[3]
		}
	}
	alpha := average[3] < float64(count)
	limit := 7.
	if alpha {
		limit = 5
	}
	lx, ly := max(1, int(math.Round(limit*float64(w)/float64(max(w, h))))), max(1, int(math.Round(limit*float64(h)/float64(max(w, h)))))
	channels := [4][]float64{}
	for c := range channels {
		channels[c] = make([]float64, count)
	}
	for i := 0; i < count; i++ {
		a := float64(input.Pix[i*4+3]) / 255
		r := average[0]*(1-a) + a/255*float64(input.Pix[i*4])
		g := average[1]*(1-a) + a/255*float64(input.Pix[i*4+1])
		b := average[2]*(1-a) + a/255*float64(input.Pix[i*4+2])
		channels[0][i] = (r + g + b) / 3
		channels[1][i] = (r+g)/2 - b
		channels[2][i] = r - g
		channels[3][i] = a
	}
	type channel struct {
		dc, scale float64
		ac        []float64
	}
	encode := func(values []float64, nx, ny int) channel {
		result := channel{}
		for cy := 0; cy < ny; cy++ {
			for cx := 0; cx*ny < nx*(ny-cy); cx++ {
				value := 0.
				for y := 0; y < h; y++ {
					fy := math.Cos(math.Pi / float64(h) * float64(cy) * (float64(y) + .5))
					for x := 0; x < w; x++ {
						value += values[x+y*w] * math.Cos(math.Pi/float64(w)*float64(cx)*(float64(x)+.5)) * fy
					}
				}
				value /= float64(count)
				if cx != 0 || cy != 0 {
					result.ac = append(result.ac, value)
					result.scale = math.Max(result.scale, math.Abs(value))
				} else {
					result.dc = value
				}
			}
		}
		if result.scale > 0 {
			for i := range result.ac {
				result.ac[i] = .5 + .5/result.scale*result.ac[i]
			}
		}
		return result
	}
	l, p, q := encode(channels[0], max(3, lx), max(3, ly)), encode(channels[1], 3, 3), encode(channels[2], 3, 3)
	a := channel{}
	if alpha {
		a = encode(channels[3], 5, 5)
	}
	round := func(v float64) int { return int(math.Floor(v + .5)) }
	h24 := round(63*l.dc) | (round(31.5+31.5*p.dc) << 6) | (round(31.5+31.5*q.dc) << 12) | (round(31*l.scale) << 18)
	if alpha {
		h24 |= 1 << 23
	}
	short := lx
	if w > h {
		short = ly
	}
	h16 := short | (round(63*p.scale) << 3) | (round(63*q.scale) << 9)
	if w > h {
		h16 |= 1 << 15
	}
	hash := []byte{byte(h24), byte(h24 >> 8), byte(h24 >> 16), byte(h16), byte(h16 >> 8)}
	values := []channel{l, p, q}
	if alpha {
		hash = append(hash, byte(round(15*a.dc)|(round(15*a.scale)<<4)))
		values = append(values, a)
	}
	start, index := len(hash), 0
	for _, channel := range values {
		for _, v := range channel.ac {
			position := start + index/2
			if position == len(hash) {
				hash = append(hash, 0)
			}
			hash[position] |= byte(round(15*v) << ((index & 1) * 4))
			index++
		}
	}
	return base64.StdEncoding.EncodeToString(hash)
}

func simMediaMIME(name string, data []byte) string {
	if mime := http.DetectContentType(data); mime != "application/octet-stream" {
		return mime
	}
	switch strings.ToLower(filepath.Ext(name)) {
	case ".caf", ".m4a", ".mp3", ".wav":
		return "audio/mp4"
	case ".mov", ".mp4":
		return "video/mp4"
	}
	return "application/octet-stream"
}

func MediaFixture(kind string) []byte {
	encoded := map[string]string{"video": simVideo, "audio": simAudio}[kind]
	if encoded == "" {
		return nil
	}
	data, _ := base64.StdEncoding.DecodeString(encoded)
	return append(data, 0, 0, 0, 8, 'f', 'r', 'e', 'e')
}
