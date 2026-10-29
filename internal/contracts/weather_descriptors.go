//go:build lab

package contracts

func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "weather.current", Version: 1, Kind: QueryOp, Source: "system", Scope: "system:read", Input: Query{}, Output: QueryResult{}, Description: "Lab-only synthetic weather at latitude and longitude; never live weather"}}
	})
}
