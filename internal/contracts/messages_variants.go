package contracts

import (
	"slices"
	"time"
	"unicode/utf16"
)

type MessageFormatting struct {
	Location int      `json:"utf16_location"`
	Length   int      `json:"utf16_length"`
	Styles   []string `json:"styles,omitempty"`
	Effect   string   `json:"effect,omitempty"`
}
type MessageSendVariant struct {
	Kind         string              `json:"kind" enum:"schedule,effect,formatting,mention,audio"`
	ScheduledFor string              `json:"scheduled_for,omitempty"`
	Effect       string              `json:"effect,omitempty"`
	Formatting   []MessageFormatting `json:"formatting,omitempty"`
	HandleID     string              `json:"handle_id,omitempty"`
	Location     int                 `json:"utf16_location,omitempty"`
	Length       int                 `json:"utf16_length,omitempty"`
}

var MessageVariantCapabilities = []string{"messages.send_later", "messages.send_effect", "messages.send_formatted", "messages.send_mention", "messages.send_audio", "messages.keep_audio"}
var MessageBubbleEffects = []string{"slam", "loud", "gentle", "invisible-ink", "echo", "spotlight", "balloons", "confetti", "love", "lasers", "fireworks", "celebration"}
var MessageTextEffects = []string{"big", "small", "shake", "nod", "explode", "ripple", "bloom", "jitter"}
var MessageTextStyles = []string{"bold", "italic", "underline", "strikethrough"}

func MessageVariantCapability(variant *MessageSendVariant) string {
	if variant == nil {
		return ""
	}
	return map[string]string{"schedule": "messages.send_later", "effect": "messages.send_effect", "formatting": "messages.send_formatted", "mention": "messages.send_mention", "audio": "messages.send_audio"}[variant.Kind]
}
func validUTF16Range(text string, location, length int) bool {
	units := utf16.Encode([]rune(text))
	if location < 0 || length < 1 || location > len(units) || length > len(units)-location {
		return false
	}
	end := location + length
	return !(location > 0 && location < len(units) && utf16.IsSurrogate(rune(units[location-1])) && units[location] >= 0xDC00 && units[location] <= 0xDFFF) && !(end < len(units) && end > 0 && units[end-1] >= 0xD800 && units[end-1] <= 0xDBFF && units[end] >= 0xDC00 && units[end] <= 0xDFFF)
}
func ValidateMessageSendVariant(args MessagesArgs, now time.Time) error {
	v := args.Variant
	if v == nil {
		return nil
	}
	if len(utf16.Encode([]rune(args.Text))) > 512 {
		return ErrTooLarge
	}
	if MessageVariantCapability(v) == "" || args.ResendOf != "" || args.ChatID == "" || len(args.Handles) != 0 {
		return ErrInvalid
	}
	if v.Kind == "audio" {
		if args.Text != "" || len(args.Files) != 1 {
			return ErrInvalid
		}
	} else if args.Text == "" || len(args.Files) != 0 {
		return ErrInvalid
	}
	if (v.ScheduledFor != "") != (v.Kind == "schedule") || (v.Effect != "") != (v.Kind == "effect") || (len(v.Formatting) > 0) != (v.Kind == "formatting") || (v.HandleID != "") != (v.Kind == "mention") || v.Kind != "mention" && (v.Location != 0 || v.Length != 0) {
		return ErrInvalid
	}
	switch v.Kind {
	case "schedule":
		date, err := time.Parse(time.RFC3339Nano, v.ScheduledFor)
		if err != nil || !date.After(now) || date.After(now.Add(14*24*time.Hour)) {
			return ErrInvalid
		}
	case "effect":
		if !slices.Contains(MessageBubbleEffects, v.Effect) {
			return ErrInvalid
		}
	case "formatting":
		if len(v.Formatting) > 32 {
			return ErrInvalid
		}
		actions := 0
		for _, f := range v.Formatting {
			actions += len(f.Styles)
			if f.Effect != "" {
				actions++
			}
			if actions > 12 {
				return ErrTooLarge
			}
			if !validUTF16Range(args.Text, f.Location, f.Length) || len(f.Styles) > 4 || len(f.Styles) == 0 && f.Effect == "" || f.Effect != "" && !slices.Contains(MessageTextEffects, f.Effect) {
				return ErrInvalid
			}
			seen := map[string]bool{}
			for _, style := range f.Styles {
				if seen[style] || !slices.Contains(MessageTextStyles, style) {
					return ErrInvalid
				}
				seen[style] = true
			}
		}
	case "mention":
		if !validUTF16Range(args.Text, v.Location, v.Length) {
			return ErrInvalid
		}
	}
	return nil
}
