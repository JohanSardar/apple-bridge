package state

import "encoding/json"

func threadSelection(chat, originator string) (string, []any) {
	if originator == "" {
		return "msg_by_chat WHERE chat_id=?", []any{chat}
	}
	return "(SELECT * FROM msg_by_chat WHERE chat_id=? AND id=? UNION ALL SELECT * FROM msg_by_chat WHERE chat_id=? AND originator=? AND id<>?) WHERE 1=1", []any{chat, originator, chat, originator, originator}
}
func threadCursorScope(chat, originator string) string {
	if originator == "" {
		return chat
	}
	value, _ := json.Marshal([]string{chat, originator})
	return string(value)
}
