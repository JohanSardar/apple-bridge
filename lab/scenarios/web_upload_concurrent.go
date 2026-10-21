package scenarios

import (
	"bytes"
	"time"
)

func uploadConcurrentProof(c *webClient, e *Evidence, add func(string, bool)) {
	data := make([]byte, 10<<20)
	type result struct {
		status  int
		upload  uploadResult
		elapsed float64
	}
	results := make(chan result, 2)
	start := time.Now()
	for i := 0; i < 2; i++ {
		go func() {
			status, upload := uploadRequest(c, "POST", "upload", bytes.NewReader(data), nil)
			results <- result{status, upload, float64(time.Since(start)) / float64(time.Millisecond)}
		}()
	}
	values := []float64{}
	passed := true
	for i := 0; i < 2; i++ {
		result := <-results
		values = append(values, result.elapsed)
		passed = passed && result.status == 200 && result.upload.State == "ready" && result.upload.Size == int64(len(data))
		uploadRequest(c, "DELETE", "upload/"+result.upload.ID, nil, nil)
	}
	e.Metrics["two_concurrent_10_MiB_completion_ms"] = values
	e.Metrics["two_concurrent_10_MiB_wall_ms"] = float64(time.Since(start)) / float64(time.Millisecond)
	add("two_concurrent_ten_MiB_uploads_verified_ready", passed)
}
