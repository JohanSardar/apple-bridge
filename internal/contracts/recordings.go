package contracts

const VoiceMemos Source = "voice_memos"
const Voicemail Source = "voicemail"

func init() {
	RegisterSource(VoiceMemos, "Voice Memos", true)
	RegisterSource(Voicemail, "Voicemail", true)
	Collections["voice_memos.recording"] = "recording"
	Collections["voicemail.message"] = "message"
	messagesShape["voice_memos.recording"] = map[string]fieldShape{"title": requiredField(shapeString), "date": requiredField(shapeTime), "duration": requiredField(shapeNumber), "folder": requiredField(shapeString), "recently_deleted": requiredField(shapeBool), "transcript_state": requiredField(shapeString), "audio_state": requiredField(shapeString)}
	messagesShape["voicemail.message"] = map[string]fieldShape{"sender": requiredField(shapeString), "date": requiredField(shapeTime), "duration": requiredField(shapeNumber), "read": requiredField(shapeBool), "mailbox": requiredField(shapeString), "type": requiredField(shapeString), "transcript_state": requiredField(shapeString), "audio_state": requiredField(shapeString)}
}
