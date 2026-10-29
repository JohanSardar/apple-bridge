package contracts

const Phone Source = "phone"

func init() {
	Collections["phone.call"] = "call"
	messagesShape["phone.call"] = map[string]fieldShape{
		"direction": requiredField(shapeString), "handle": requiredField(shapeString), "name": requiredField(shapeString), "date": requiredField(shapeTime), "duration": requiredField(shapeNumber), "service": requiredField(shapeString), "answered": requiredField(shapeBool),
	}
}
