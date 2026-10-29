package contracts

func init() {
	RegisterSource("focus", "Focus", true)
	Collections["focus.status"] = "fcs"
	messagesShape["focus.status"] = map[string]fieldShape{"focused": nullableField(shapeBool), "name": nullableField(shapeString), "authorization": requiredField(shapeString)}
}
