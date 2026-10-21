package demo

import (
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func Populate(w *world.World, dataset Dataset) error {
	previous := w.Control.Timeout
	w.Control.Timeout = 2 * time.Minute
	defer func() { w.Control.Timeout = previous }()
	return w.ControlAction("/dataset", dataset.Dataset)
}
