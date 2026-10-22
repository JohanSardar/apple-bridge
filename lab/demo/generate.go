package demo

import (
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"math/rand/v2"
	"time"
)

func Generate(options Options) Dataset {
	d := Dataset{Dataset: simapple.Dataset{Blobs: map[string][]byte{}, LivePhotos: map[string]string{}}, Seed: options.Seed, Counts: map[string]int{}}
	rng := rand.New(rand.NewPCG(uint64(options.Seed), 0x9e3779b97f4a7c15))
	d.people(rng)
	media := mediaAssets(rng)
	for c := 0; c < 40; c++ {
		count := 100
		if c == 0 {
			count = 200
		}
		if c == 1 {
			count = 1000
		}
		if c == 39 && options.Huge {
			count = 100000
		}
		for i := 0; i < count; i++ {

			key := messageKey(c, i)
			fields := messageFields(c, i, count, rng)
			sent := fields["sent_at"].(time.Time)
			if c == 0 {
				d.showcase(i, key, sent, fields, media)
				d.richShowcase(i, key, sent, fields, media)
			}
			if i == count-1 {
				d.preview(c, key, sent, fields, media)
			}
			d.put("messages.message", key, fields)
		}
	}
	d.reactions()
	d.reminders()
	d.calendars()
	return d
}
