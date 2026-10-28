package simapple

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func datasetAlias(o contracts.Observation) string {
	if contracts.CollectionSource(o.Collection) == contracts.Mail && o.ScopedAlias != "" {
		return o.ScopedAlias
	}
	return o.Collection + "/" + o.NativeKey
}
