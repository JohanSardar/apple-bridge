package simapple

import (
	"bytes"
	"crypto/sha256"
	"fmt"
	"image"
	"image/jpeg"
	"image/png"
	"math"
)

func posterDimensions(width, height int) (int, int) {
	if width <= 0 || height <= 0 {
		return 0, 0
	}
	ratio := float64(max(width, height)) / float64(min(width, height))
	short := max(1, min(360, int(math.Floor(math.Sqrt(1500000/ratio)))))
	long := max(1, int(math.Floor(math.Min(1500000, float64(short)*ratio))))
	if width >= height {
		return long, short
	}
	return short, long
}

func posterResize(input image.Image) *image.NRGBA {
	bounds := input.Bounds()
	width, height := posterDimensions(bounds.Dx(), bounds.Dy())
	output := image.NewNRGBA(image.Rect(0, 0, width, height))
	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			output.Set(x, y, input.At(bounds.Min.X+x*bounds.Dx()/width, bounds.Min.Y+y*bounds.Dy()/height))
		}
	}
	return output
}

func (f *fakeApple) posterVariant(poster *image.NRGBA) map[string]any {
	var buffer bytes.Buffer
	mime := "image/jpeg"
	if poster.Opaque() {
		jpeg.Encode(&buffer, poster, &jpeg.Options{Quality: 80})
	} else {
		mime = "image/png"
		png.Encode(&buffer, poster)
	}
	digest := fmt.Sprintf("%x", sha256.Sum256(buffer.Bytes()))
	f.blobs[digest] = buffer.Bytes()
	return map[string]any{"digest": digest, "state": "present", "size": buffer.Len(), "mime": mime, "width": poster.Bounds().Dx(), "height": poster.Bounds().Dy()}
}

func (f *fakeApple) livePoster(original []byte) map[string]any {
	still, _, err := image.Decode(bytes.NewReader(original))
	if err != nil {
		return nil
	}
	return f.posterVariant(posterResize(still))
}
