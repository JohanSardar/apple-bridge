package simapple

import (
	"image"
	"image/color"
)

func sourceVideoPoster(width, height int, source bool) *image.NRGBA {
	a, b := width, height
	for b != 0 {
		a, b = b, a%b
	}
	w, h := width/a, height/a
	factor := min(a, 320/max(w, h))
	if factor > 0 {
		w, h = w*factor, h*factor
	} else {
		w, h = max(1, width*320/max(width, height)), max(1, height*320/max(width, height))
	}
	output := image.NewNRGBA(image.Rect(0, 0, w, h))
	side := min(w, h) * 4 / 5
	for y := 0; y < h; y++ {
		for x := 0; x < w; x++ {
			pixel := color.NRGBA{R: 40, G: 120, B: 200, A: 255}
			if source {
				pixel = color.NRGBA{G: 76, B: 200, A: 255}
				if x >= (w-side)/2 && x < (w+side)/2 && y >= (h-side)/2 && y < (h+side)/2 {
					pixel = color.NRGBA{R: 255, A: 255}
				}
			}
			output.SetNRGBA(x, y, pixel)
		}
	}
	return output
}
