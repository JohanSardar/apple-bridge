package contracts

func MacDescriptors() []OperationDescriptor { return nil }

func init() {
	RegisterSource("mac", "Mac", true)
	registerDescriptors(MacDescriptors)
	for collection, prefix := range map[string]string{"mac.audio_device": "mad", "mac.display": "mdp", "mac.power": "mpw", "mac.network": "mnw", "mac.app": "map"} {
		Collections[collection] = prefix
	}
	messagesShape["mac.audio_device"] = map[string]fieldShape{"name": requiredField(shapeString), "input": requiredField(shapeBool), "output": requiredField(shapeBool), "default": requiredField(shapeBool), "volume": optionalField(shapeNumber), "muted": optionalField(shapeBool)}
	messagesShape["mac.display"] = map[string]fieldShape{"appearance": requiredField(shapeString), "displays": requiredField(shapeArray)}
	messagesShape["mac.power"] = map[string]fieldShape{"battery_percent": optionalField(shapeNumber), "charging": requiredField(shapeBool), "source": requiredField(shapeString), "low_power_mode": requiredField(shapeBool), "time_remaining": optionalField(shapeNumber), "keep_awake_held": requiredField(shapeBool)}
	messagesShape["mac.network"] = map[string]fieldShape{"ssid": optionalField(shapeString), "wifi_power": optionalField(shapeBool), "connected": requiredField(shapeBool), "interface": optionalField(shapeString), "vpn": requiredField(shapeBool)}
	messagesShape["mac.app"] = map[string]fieldShape{"bundle_id": requiredField(shapeString), "name": requiredField(shapeString), "running": requiredField(shapeBool), "frontmost": requiredField(shapeBool), "hidden": requiredField(shapeBool)}
}
