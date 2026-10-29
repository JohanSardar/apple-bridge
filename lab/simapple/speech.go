package simapple

import (
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func simulateAudioQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "speech.synthesize" {
		return "", nil, false
	}
	var args struct {
		Text string `json:"text"`
	}
	if contracts.DecodeInput(raw, &args) != nil || args.Text == "" || len(args.Text) > 2048 {
		return "invalid", nil, true
	}
	wav := make([]byte, 364)
	copy(wav, "RIFF")
	binary.LittleEndian.PutUint32(wav[4:], 356)
	copy(wav[8:], "WAVEfmt ")
	binary.LittleEndian.PutUint32(wav[16:], 16)
	binary.LittleEndian.PutUint16(wav[20:], 1)
	binary.LittleEndian.PutUint16(wav[22:], 1)
	binary.LittleEndian.PutUint32(wav[24:], 16000)
	binary.LittleEndian.PutUint32(wav[28:], 32000)
	binary.LittleEndian.PutUint16(wav[32:], 2)
	binary.LittleEndian.PutUint16(wav[34:], 16)
	copy(wav[36:], "data")
	binary.LittleEndian.PutUint32(wav[40:], 320)
	result, _ := json.Marshal(map[string]string{"media_type": "audio/wav", "data_base64": base64.StdEncoding.EncodeToString(wav)})
	return "", result, true
}
