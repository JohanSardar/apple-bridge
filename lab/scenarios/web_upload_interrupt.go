package scenarios

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"time"
)

func interruptUpload(c *webClient, id string, data []byte) (uploadResult, bool) {
	ctx, cancel := context.WithCancel(c.ctx)
	defer cancel()
	reader, writer := io.Pipe()
	defer reader.Close()
	defer writer.Close()
	request, _ := http.NewRequestWithContext(ctx, "POST", c.origin+"/web/v1/upload", reader)
	request.ContentLength = int64(len(data))
	request.Header.Set("Origin", c.origin)
	request.Header.Set("Upload-ID", id)
	request.Header.Set("Upload-Offset", "0")
	request.AddCookie(c.cookie)
	done := make(chan struct{})
	go func() {
		defer close(done)
		response, err := http.DefaultClient.Do(request)
		if err == nil {
			response.Body.Close()
		}
	}()
	go func() { io.Copy(writer, bytes.NewReader(data[:1<<20])) }()
	var out uploadResult
	accepted := false
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		status, result := uploadRequest(c, "GET", "upload/"+id, nil, nil)
		out = result
		if status == 200 && out.Offset == 1<<20 {
			accepted = true
			break
		}
		select {
		case <-ctx.Done():
			break
		case <-time.After(20 * time.Millisecond):
		}
	}
	cancel()
	reader.Close()
	writer.Close()
	select {
	case <-done:
	case <-time.After(time.Second):
	}
	return out, accepted
}
