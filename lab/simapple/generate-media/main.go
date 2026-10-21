package main

import (
	"encoding/json"
	"os"

	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "audio" {
		if _, err := os.Stdout.Write(simapple.MediaFixture("audio")); err != nil {
			panic(err)
		}
		return
	}
	dataset := demo.Generate(demo.Options{Seed: 1})
	for _, record := range dataset.Records {
		if record.Collection != "messages.attachment" {
			continue
		}
		var fields struct {
			Filename string `json:"filename"`
			Blob     struct {
				Digest string `json:"digest"`
			} `json:"blob"`
		}
		if json.Unmarshal(record.Data, &fields) == nil && fields.Filename == "demo-square.png" {
			if _, err := os.Stdout.Write(dataset.Blobs[fields.Blob.Digest]); err != nil {
				panic(err)
			}
			return
		}
	}
	panic("square demo frame missing")
}
