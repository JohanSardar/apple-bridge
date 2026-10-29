package contracts

func init() {
	RegisterSource("weather", "Weather", true)
	registerDescriptors(func() []OperationDescriptor {
		var descriptors []OperationDescriptor
		for _, name := range []string{"weather.current", "weather.hourly", "weather.daily"} {
			descriptors = append(descriptors, OperationDescriptor{Name: name, Version: 1, Kind: QueryOp, Source: "weather", Scope: "weather:read", Input: Query{}, Output: QueryResult{}, Description: "Transient WeatherKit conditions or forecast with Celsius units and Apple attribution"})
		}
		return descriptors
	})
}
