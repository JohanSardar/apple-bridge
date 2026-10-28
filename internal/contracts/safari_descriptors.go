package contracts

const Safari Source = "safari"

func init() {
	for collection, prefix := range map[string]string{"safari.history": "sfh", "safari.bookmark": "sfb", "safari.reading_list": "sfr", "safari.cloud_tab": "sfc"} {
		Collections[collection] = prefix
	}
	RegisterSource(Safari, "Safari", true)
	messagesShape["safari.history"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "last_visit": requiredField(shapeTime), "visit_count": requiredField(shapeNumber)}
	messagesShape["safari.bookmark"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "folder_path": requiredField(shapeString)}
	messagesShape["safari.reading_list"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "added": nullableField(shapeTime), "read": requiredField(shapeBool)}
	messagesShape["safari.cloud_tab"] = map[string]fieldShape{"url": requiredField(shapeString), "title": requiredField(shapeString), "device": requiredField(shapeString)}
}
