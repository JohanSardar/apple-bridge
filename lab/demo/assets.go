package demo

import (
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"math/rand/v2"
)

type mediaAsset struct {
	name, mime string
	data       []byte
}

func mediaAssets(rng *rand.Rand) []mediaAsset {
	return []mediaAsset{
		{"demo-portrait.png", "image/png", picture(rng.Uint64(), 240, 400, false)},
		{"demo-landscape.png", "image/png", picture(rng.Uint64(), 640, 400, false)},
		{"demo-square.png", "image/png", picture(rng.Uint64(), 360, 360, false)},
		{"demo-panorama.png", "image/png", picture(rng.Uint64(), 1024, 256, false)},
		{"demo-transparent.png", "image/png", picture(rng.Uint64(), 256, 256, true)},
		{"demo-motion.gif", "image/gif", animation()},
		{"demo-video.mp4", "video/mp4", simapple.MediaFixture("video")},
		{"demo-audio.m4a", "audio/mp4", simapple.MediaFixture("audio")},
		{"demo-field-notes.pdf", "application/pdf", document()},
		{"demo-contact.vcf", "text/vcard", []byte("BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Demo Aster Example\r\nN:Example;Demo Aster;;;\r\nTEL:+1 555 0100\r\nEMAIL:demo.person00@example.com\r\nEND:VCARD\r\n")},
	}
}
