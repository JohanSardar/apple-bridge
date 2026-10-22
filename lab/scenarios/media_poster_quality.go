package scenarios

import (
	"image"
	"math"
)

func posterEdgeContrast(poster image.Image) float64 {
	bounds := poster.Bounds()
	scale := float64(bounds.Dy()) / 360
	edge := float64(bounds.Min.X) + float64(bounds.Dx())/2 - float64(bounds.Dy())*0.4
	sample := func(x float64, y int) float64 {
		x -= 0.5
		left := max(bounds.Min.X, min(bounds.Max.X-1, int(math.Floor(x))))
		right := min(bounds.Max.X-1, left+1)
		fraction := x - math.Floor(x)
		lr, _, lb, _ := poster.At(left, y).RGBA()
		rr, _, rb, _ := poster.At(right, y).RGBA()
		return (float64(lr)-float64(lb))*(1-fraction) + (float64(rr)-float64(rb))*fraction
	}
	contrast := 0.0
	for offset := -3; offset <= 3; offset++ {
		delta := 0.0
		for row := -2; row <= 2; row++ {
			y := max(bounds.Min.Y, min(bounds.Max.Y-1, bounds.Min.Y+bounds.Dy()/2+row))
			center := edge + float64(offset)*scale
			delta += (sample(center+1.5*scale, y) - sample(center-1.5*scale, y)) / 131070
		}
		contrast = max(contrast, delta/5)
	}
	return contrast
}
