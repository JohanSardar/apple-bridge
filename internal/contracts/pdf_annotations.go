package contracts

import (
	"encoding/base64"
	"encoding/json"
	"math"
)

type PDFAnnotation struct {
	Page   int     `json:"page"`
	Type   string  `json:"type"`
	Text   string  `json:"text"`
	X      float64 `json:"x"`
	Y      float64 `json:"y"`
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

type PDFAnnotateArgs struct {
	Data        string          `json:"data_base64,omitempty"`
	File        string          `json:"file,omitempty"`
	AllowFile   bool            `json:"allow_file,omitempty"`
	Blob        json.RawMessage `json:"blob,omitempty"`
	Annotations []PDFAnnotation `json:"annotations"`
}

func ValidatePDFAnnotate(c Command) error {
	var a PDFAnnotateArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || c.ExpectedVersion != 0 || DecodeInput(c.Args, &a) != nil || len(a.Annotations) == 0 || len(a.Annotations) > 100 {
		return ErrInvalid
	}
	inputs := 0
	if a.Data != "" {
		inputs++
		data, err := base64.StdEncoding.DecodeString(a.Data)
		if err != nil || len(data) == 0 || len(data) > 8<<20 {
			return ErrInvalid
		}
	}
	if a.File != "" {
		inputs++
		if !a.AllowFile || a.File[0] != '/' {
			return ErrInvalid
		}
	}
	if len(a.Blob) > 0 {
		inputs++
	}
	if inputs != 1 {
		return ErrInvalid
	}
	for _, n := range a.Annotations {
		if n.Page < 1 || n.Page > 1000 || n.Type != "note" && n.Type != "highlight" || len(n.Text) > 4096 || n.Type == "note" && n.Text == "" || n.X < 0 || n.Y < 0 || n.Width <= 0 || n.Height <= 0 {
			return ErrInvalid
		}
		for _, v := range []float64{n.X, n.Y, n.Width, n.Height} {
			if math.IsNaN(v) || math.IsInf(v, 0) || v > 100000 {
				return ErrInvalid
			}
		}
	}
	return nil
}
