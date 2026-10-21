package scenarios

import (
	"bytes"
	"fmt"
)

func uploadReplicaResume(c *webClient, data []byte, add func(string, bool)) (int, uploadResult) {
	status, started := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(len(data))})
	add("replica_resumable_upload_initialized", status == 200 && started.ID != "" && started.Offset == 0 && started.State == "uploading")
	if status != 200 || started.ID == "" {
		return status, started
	}
	partial, accepted := interruptUpload(c, started.ID, data)
	add("replica_http_interruption_preserves_prefix", accepted && partial.ID == started.ID && partial.Offset == 1<<20 && partial.State == "uploading")
	if !accepted {
		return 0, partial
	}
	status, discovered := uploadRequest(c, "GET", "upload/"+started.ID, nil, nil)
	add("replica_offset_discovered_without_lease_refresh", status == 200 && discovered.Offset == partial.Offset && discovered.Expires.Equal(partial.Expires))
	return uploadRequest(c, "POST", "upload", bytes.NewReader(data[partial.Offset:]), map[string]string{"Upload-ID": started.ID, "Upload-Offset": fmt.Sprint(partial.Offset)})
}
