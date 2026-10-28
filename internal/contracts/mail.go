package contracts

const Mail Source = "mail"

func init() {
	RegisterSource(Mail, "Mail", true)
	for collection, prefix := range map[string]string{"mail.account": "mac", "mail.mailbox": "mbx", "mail.message": "mml", "mail.attachment": "mat"} {
		Collections[collection] = prefix
	}
	messagesShape["mail.account"] = map[string]fieldShape{"name": requiredField(shapeString), "kind": requiredField(shapeString), "addresses": requiredField(shapeArray)}
	messagesShape["mail.mailbox"] = map[string]fieldShape{"account_id": requiredField(shapeString), "name": requiredField(shapeString), "path": requiredField(shapeString), "role": requiredField(shapeString), "unread_count": requiredField(shapeNumber), "message_count": requiredField(shapeNumber)}
	messagesShape["mail.message"] = map[string]fieldShape{"account_id": requiredField(shapeString), "mailbox_ids": requiredField(shapeArray), "subject": requiredField(shapeString), "from": requiredField(shapeString), "to": nullableField(shapeArray), "cc": nullableField(shapeArray), "sent_at": nullableField(shapeTime), "received_at": requiredField(shapeTime), "read": requiredField(shapeBool), "flagged": nullableField(shapeBool), "flag_color": nullableField(shapeNumber), "has_attachments": nullableField(shapeBool), "size": nullableField(shapeNumber), "snippet": nullableField(shapeString), "body_state": nullableField(shapeString), "message_id": requiredField(shapeString)}
	messagesShape["mail.attachment"] = map[string]fieldShape{"message_id": requiredField(shapeString), "filename": requiredField(shapeString), "mime_type": requiredField(shapeString), "size": nullableField(shapeNumber), "blob": requiredField(shapeObject)}
}
