package simapple

import "testing"

func TestPosterDimensions(t *testing.T) {
	for _, item := range []struct{ width, height, wantWidth, wantHeight int }{
		{1920, 200, 3456, 360}, {200, 1920, 360, 3456}, {320, 180, 640, 360}, {180, 320, 360, 640}, {1, 1, 360, 360}, {100000, 1, 300000, 3}, {1, 100000, 3, 300000}, {0, 200, 0, 0}, {200, 0, 0, 0}, {-1, 200, 0, 0},
	} {
		width, height := posterDimensions(item.width, item.height)
		if width != item.wantWidth || height != item.wantHeight {
			t.Errorf("posterDimensions(%d,%d) = %dx%d, want %dx%d", item.width, item.height, width, height, item.wantWidth, item.wantHeight)
		}
		if width*height > 1500000 {
			t.Errorf("poster dimensions exceed budget: %dx%d", width, height)
		}
	}
}
