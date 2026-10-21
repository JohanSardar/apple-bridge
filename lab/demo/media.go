package demo

import (
	"bytes"
	"fmt"
	"image"
	"image/color"
	"image/gif"
	"image/png"
	"math"
	"math/rand/v2"
)

func picture(seed uint64, width, height int, transparent bool) []byte {
	rng := rand.New(rand.NewPCG(seed, seed^0x85ebca6b))
	base := [3]float64{80 + rng.Float64()*100, 90 + rng.Float64()*100, 130 + rng.Float64()*100}
	output := image.NewNRGBA(image.Rect(0, 0, width, height))
	for y := 0; y < height; y++ {
		for x := 0; x < width; x++ {
			u, v := float64(x)/float64(width), float64(y)/float64(height)
			shape := math.Exp(-12 * ((u-.35)*(u-.35) + (v-.4)*(v-.4)))
			alpha := 255.
			if transparent {
				alpha = 255 * math.Min(1, math.Max(0, (.44-math.Hypot(u-.5, v-.5))*30))
			}
			output.SetNRGBA(x, y, color.NRGBA{R: uint8(math.Min(255, base[0]+50*u+35*shape)), G: uint8(math.Min(255, base[1]+35*v+45*shape)), B: uint8(math.Min(255, base[2]+25*(1-u))), A: uint8(alpha)})
		}
	}
	var buffer bytes.Buffer
	png.Encode(&buffer, output)
	return buffer.Bytes()
}

func animation() []byte {
	palette := color.Palette{color.NRGBA{R: 90, G: 150, B: 210, A: 255}, color.NRGBA{R: 190, G: 130, B: 210, A: 255}}
	frames := []*image.Paletted{}
	for i := 0; i < 2; i++ {
		frame := image.NewPaletted(image.Rect(0, 0, 96, 96), palette)
		for y := 0; y < 96; y++ {
			for x := 0; x < 96; x++ {
				frame.SetColorIndex(x, y, uint8((x/24+y/24+i)%2))
			}
		}
		frames = append(frames, frame)
	}
	var buffer bytes.Buffer
	gif.EncodeAll(&buffer, &gif.GIF{Image: frames, Delay: []int{30, 30}, LoopCount: 0})
	return buffer.Bytes()
}

func document() []byte {
	objects := []string{"<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 240] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"}
	content := "BT /F1 18 Tf 32 180 Td (Synthetic Demo Field Notes) Tj ET"
	objects = append(objects, fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(content), content))
	var buffer bytes.Buffer
	buffer.WriteString("%PDF-1.4\n")
	offsets := []int{0}
	for i, object := range objects {
		offsets = append(offsets, buffer.Len())
		fmt.Fprintf(&buffer, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	start := buffer.Len()
	fmt.Fprintf(&buffer, "xref\n0 %d\n0000000000 65535 f \n", len(offsets))
	for _, offset := range offsets[1:] {
		fmt.Fprintf(&buffer, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&buffer, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(offsets), start)
	return buffer.Bytes()
}
