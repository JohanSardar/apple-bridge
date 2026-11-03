//go:build !lab

package web

func labClientMetadata() *ClientMetadataFetcher { return &ClientMetadataFetcher{} }
