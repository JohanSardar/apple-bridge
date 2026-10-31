package simapple

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"sync"
	"time"
)

type querySession struct {
	ctx     context.Context
	fake    *fakeApple
	wire    *applepeer.Wire
	mu      sync.Mutex
	running map[string]context.CancelFunc
}

func newQuerySession(ctx context.Context, f *fakeApple, w *applepeer.Wire) *querySession {
	return &querySession{ctx: ctx, fake: f, wire: w, running: map[string]context.CancelFunc{}}
}
func (s *querySession) receive(f contracts.IPCFrame) bool {
	if f.T != "Query" && f.T != "QueryCancel" {
		return false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if f.T == "QueryCancel" {
		if cancel := s.running[f.ID]; cancel != nil {
			cancel()
		}
		return true
	}
	if s.running[f.ID] != nil {
		return true
	}
	s.fake.mu.Lock()
	s.fake.effects[f.ID] = 1
	s.fake.mu.Unlock()
	ctx, cancel := context.WithTimeout(s.ctx, time.Duration(f.DeadlineMS)*time.Millisecond)
	s.running[f.ID] = cancel
	go func() {
		defer func() { s.fake.mu.Lock(); s.fake.effects[f.ID] = 0; s.fake.mu.Unlock() }()
		defer cancel()
		defer func() { s.mu.Lock(); delete(s.running, f.ID); s.mu.Unlock() }()
		out := contracts.IPCFrame{T: "QueryResult", ID: f.ID, ResultJSON: f.Args, Blobs: []contracts.QueryBlob{}}
		var args struct {
			MS   int `json:"ms"`
			Size int `json:"size"`
		}
		decodeErr := contracts.DecodeInput(f.Args, &args)
		var fields map[string]json.RawMessage
		json.Unmarshal(f.Args, &fields)
		field := "ms"
		if f.Op == "test.blob" {
			field = "size"
		}
		if (f.Op == "test.sleep" || f.Op == "test.blob") && (decodeErr != nil || len(fields) != 1 || fields[field] == nil || args.MS < 0 || args.MS > 60000 || args.Size < 0 || args.Size > 64<<20) {
			_, _ = s.wire.Send(contracts.IPCFrame{T: "QueryError", ID: f.ID, Code: "invalid"})
			return
		}
		switch f.Op {
		case "shortcuts.actions":
			var fields map[string]json.RawMessage
			if json.Unmarshal(f.Args, &fields) != nil || fields == nil || len(fields) != 0 {
				out.T = "QueryError"
				out.Code = "invalid"
			} else {
				out.ResultJSON = json.RawMessage(`{"items":[{"app":"com.example.synthetic","identifier":"Export","title":"Export"}],"complete":true,"schema_guaranteed":false}`)
			}
		case "weather.current":
			result, weatherErr := syntheticWeather(ctx, f.Args)
			out.ResultJSON = result
			if weatherErr != nil {
				out.T = "QueryError"
				out.Code = "invalid"
			}
		case "test.echo":
		case "test.sleep":
			select {
			case <-ctx.Done():
				out.T = "QueryError"
				out.Code = "deadline"
			case <-time.After(time.Duration(args.MS) * time.Millisecond):
			}
		case "test.blob":
			if args.Size < 0 || args.Size > 64<<20 {
				out.T = "QueryError"
				out.Code = "invalid"
				break
			}
			data := make([]byte, args.Size)
			for i := range data {
				data[i] = byte(i % 251)
			}
			sum := sha256.Sum256(data)
			digest := hex.EncodeToString(sum[:])
			s.fake.mu.Lock()
			s.fake.blobs[digest] = data
			s.fake.mu.Unlock()
			out.ResultJSON = json.RawMessage(`{}`)
			out.Blobs = []contracts.QueryBlob{{Digest: digest, Size: int64(len(data)), Handle: digest}}
		case "mail.body":
			var bodyArgs struct {
				MessageID string `json:"message_id"`
			}
			if contracts.DecodeInput(f.Args, &bodyArgs) != nil || bodyArgs.MessageID == "" {
				out.T, out.Code = "QueryError", "invalid"
			} else {
				out.ResultJSON = json.RawMessage(`{"state":"downloaded","plain_text":"Hello fixture","html":""}`)
			}
		case "mac.bluetooth_devices":
			out.ResultJSON = json.RawMessage(`{"devices":[{"name":"Bridge Test Headphones","connected":true,"battery_percent":75}]}`)
		case "mac.time_machine_status":
			out.ResultJSON = json.RawMessage(`{"running":false,"phase":"idle"}`)
		case "mac.recognize_song":
			var args struct {
				AudioBlob string `json:"audio_blob"`
			}
			if contracts.DecodeInput(f.Args, &args) != nil || args.AudioBlob != "YnJpZGdlLWxhYi1hdWRpbw==" {
				out.T = "QueryError"
				out.Code = "invalid"
			} else {
				out.ResultJSON = json.RawMessage(`{"recognized":true,"title":"Bridge Test Song","artist":"Bridge Test Artist"}`)
			}
		case "mac.network_check":
			out.ResultJSON = json.RawMessage(`{"connected":true,"dns":true,"ping":"limited: ICMP unavailable","interfaces":[{"name":"en0","received_bytes":123,"sent_bytes":456}]}`)
		case "mac.clipboard_read":
			s.fake.mu.Lock()
			out.ResultJSON, _ = json.Marshal(map[string]any{"text": s.fake.clipboard})
			s.fake.mu.Unlock()
		case "helper.version":
			out.ResultJSON = json.RawMessage(`{"version":"sim","protocol":"ipc/1"}`)
		default:
			if f.Op == "music.now_playing" {
				var fields map[string]json.RawMessage
				if contracts.DecodeInput(f.Args, &fields) != nil || len(fields) != 0 {
					out.T = "QueryError"
					out.Code = "invalid"
				} else {
					s.fake.mu.Lock()
					player := s.fake.music.player()
					delete(player, "id")
					out.ResultJSON, _ = json.Marshal(player)
					s.fake.mu.Unlock()
				}
				break
			}
			if code, result, handled := simulateMusicQuery(f.Op, f.Args); handled {
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				} else {
					out.ResultJSON = result
				}
				break
			}
			if code, result, blobs, handled := s.fake.mapsQuery(f.Op, f.Args); handled {
				out.Blobs = blobs
				out.ResultJSON = result
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			if photos, handled := s.fake.photosQuery(f); handled {
				out = photos
				break
			}
			if files, handled := s.fake.filesQuery(f); handled {
				out = files
				break
			}
			for _, handler := range []func(string, json.RawMessage) (string, json.RawMessage, bool){simulateTranscription, simulateAIQuery} {
				if code, result, handled := handler(f.Op, f.Args); handled {
					out.ResultJSON = result
					if code != "" {
						out.T = "QueryError"
						out.Code = code
					}
					_, _ = s.wire.Send(out)
					return
				}
			}
			if code, result, handled := simulateAudioQuery(f.Op, f.Args); handled {
				out.ResultJSON = result
				if f.Op == "speech.synthesize" && code == "" {
					var value struct {
						Data string `json:"data_base64"`
					}
					json.Unmarshal(result, &value)
					data, _ := base64.StdEncoding.DecodeString(value.Data)
					sum := sha256.Sum256(data)
					digest := hex.EncodeToString(sum[:])
					s.fake.mu.Lock()
					s.fake.blobs[digest] = data
					s.fake.mu.Unlock()
					out.ResultJSON = json.RawMessage(`{"media_type":"audio/wav"}`)
					out.Blobs = []contracts.QueryBlob{{Digest: digest, Size: int64(len(data)), Handle: digest}}
				}
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			if code, result, handled := s.fake.simulatePDFQuery(f.Op, f.Args); handled {
				out.ResultJSON = result
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			if code, result, handled := simulateVisionQuery(f.Op, f.Args); handled {
				out.ResultJSON = result
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			out.T = "QueryError"
			out.Code = "unsupported"
		}
		if ctx.Err() == nil || out.T == "QueryError" {
			_, _ = s.wire.Send(out)
		}
	}()
	return true
}
