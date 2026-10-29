package contracts

import (
	"bytes"
	"encoding"
	"encoding/json"
	"reflect"
	"strings"
	"time"
	"unicode/utf8"
)

const MaxQueryJSON = 1 << 20

type Query struct {
	ID         string          `json:"id,omitempty"`
	Op         string          `json:"op,omitempty"`
	Args       json.RawMessage `json:"args"`
	DeadlineMS int             `json:"deadline_ms,omitempty"`
}
type QueryBlob struct {
	Digest string `json:"digest"`
	Size   int64  `json:"size"`
	Handle string `json:"handle,omitempty"`
}
type QueryResult struct {
	ID         string          `json:"id"`
	ResultJSON json.RawMessage `json:"result_json"`
	Blobs      []QueryBlob     `json:"blobs"`
}
type QueryError struct {
	ID   string `json:"id"`
	Code string `json:"code"`
}

func MarshalQueryJSON(value any) ([]byte, error) {
	budget := MaxQueryFrame
	if err := queryJSONBudget(reflect.ValueOf(value), &budget, 0); err != nil {
		return nil, err
	}
	var buffer bytes.Buffer
	encoder := json.NewEncoder(&queryJSONWriter{buffer: &buffer})
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return nil, err
	}
	return buffer.Bytes()[:buffer.Len()-1], nil
}

type queryJSONWriter struct{ buffer *bytes.Buffer }

func (w *queryJSONWriter) Write(p []byte) (int, error) {
	if len(p) > MaxQueryFrame+1-w.buffer.Len() {
		return 0, ErrCapacity
	}
	return w.buffer.Write(p)
}

func queryJSONBudget(v reflect.Value, budget *int, depth int) error {
	if depth > 128 {
		return ErrCapacity
	}
	spend := func(n int) error {
		if n > *budget {
			return ErrCapacity
		}
		*budget -= n
		return nil
	}
	if !v.IsValid() {
		return spend(4)
	}
	if v.Kind() == reflect.Interface {
		if v.IsNil() {
			return spend(4)
		}
		return queryJSONBudget(v.Elem(), budget, depth+1)
	}
	if v.Type() == reflect.TypeFor[json.RawMessage]() {
		if v.Len() == 0 {
			return spend(4)
		}
		return spend(v.Len())
	}
	if v.Type() == reflect.TypeFor[time.Time]() {
		return spend(37)
	}
	if v.CanInterface() {
		if _, ok := v.Interface().(encoding.TextMarshaler); ok {
			return ErrInvalid
		}
		if _, ok := v.Interface().(json.Marshaler); ok {
			switch v.Interface().(type) {
			case IPCFrame, FeedFrame, feedFields, *IPCFrame, *FeedFrame:
			default:
				return ErrInvalid
			}
		}
	}
	switch v.Kind() {
	case reflect.Pointer:
		if v.IsNil() {
			return spend(4)
		}
		return queryJSONBudget(v.Elem(), budget, depth+1)
	case reflect.String:
		if err := spend(2); err != nil {
			return err
		}
		text := v.String()
		for len(text) > 0 {
			r, n := utf8.DecodeRuneInString(text)
			size := n
			if r == utf8.RuneError && n == 1 {
				size = 3
			}
			if r == '"' || r == '\\' || r == '\n' || r == '\r' || r == '\t' || r == '\b' || r == '\f' {
				size = 2
			} else if r < 32 || r == '\u2028' || r == '\u2029' {
				size = 6
			}
			if err := spend(size); err != nil {
				return err
			}
			text = text[n:]
		}
	case reflect.Struct:
		if err := spend(2); err != nil {
			return err
		}
		for i := 0; i < v.NumField(); i++ {
			field := v.Type().Field(i)
			if field.PkgPath != "" {
				continue
			}
			tag := strings.Split(field.Tag.Get("json"), ",")
			if tag[0] == "-" {
				continue
			}
			value := v.Field(i)
			if field.Name == "ResultJSON" && value.Type() == reflect.TypeFor[json.RawMessage]() && value.Len() > MaxQueryJSON {
				return ErrCapacity
			}
			if len(tag) > 1 && tag[1] == "omitempty" && queryJSONEmpty(value) {
				continue
			}
			if !field.Anonymous {
				name := tag[0]
				if name == "" {
					name = field.Name
				}
				if err := queryJSONBudget(reflect.ValueOf(name), budget, depth+1); err != nil {
					return err
				}
				if err := spend(2); err != nil {
					return err
				}
			}
			if err := queryJSONBudget(value, budget, depth+1); err != nil {
				return err
			}
		}
	case reflect.Slice, reflect.Array:
		if v.Kind() == reflect.Slice && v.IsNil() {
			return spend(4)
		}
		if v.Type().Elem().Kind() == reflect.Uint8 {
			return spend(2 + (v.Len()+2)/3*4)
		}
		if err := spend(2); err != nil {
			return err
		}
		for i := 0; i < v.Len(); i++ {
			if err := spend(1); err != nil {
				return err
			}
			if err := queryJSONBudget(v.Index(i), budget, depth+1); err != nil {
				return err
			}
		}
	case reflect.Map:
		if v.IsNil() {
			return spend(4)
		}
		if err := spend(2); err != nil {
			return err
		}
		iter := v.MapRange()
		for iter.Next() {
			if err := spend(2); err != nil {
				return err
			}
			if err := queryJSONBudget(iter.Key(), budget, depth+1); err != nil {
				return err
			}
			if err := queryJSONBudget(iter.Value(), budget, depth+1); err != nil {
				return err
			}
		}
	default:
		if v.CanInterface() {
			raw, err := json.Marshal(v.Interface())
			if err != nil {
				return err
			}
			return spend(len(raw))
		}
		return ErrInvalid
	}
	return nil
}
func queryJSONEmpty(v reflect.Value) bool {
	switch v.Kind() {
	case reflect.Array, reflect.Map, reflect.Slice, reflect.String:
		return v.Len() == 0
	case reflect.Bool, reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64, reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr, reflect.Float32, reflect.Float64, reflect.Interface, reflect.Pointer:
		return v.IsZero()
	}
	return false
}

func ForwardTimeout(raw json.RawMessage) time.Duration {
	var request struct {
		Method string `json:"method"`
		Query  *Query `json:"native_query"`
	}
	if json.Unmarshal(raw, &request) == nil && request.Method == "query" && request.Query != nil {
		ms := request.Query.DeadlineMS
		if ms == 0 {
			ms = 1000
		}
		if ms > 0 && ms <= 60000 {
			return time.Duration(ms) * time.Millisecond
		}
	}
	return 20 * time.Second
}
