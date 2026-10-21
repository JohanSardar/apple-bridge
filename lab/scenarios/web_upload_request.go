package scenarios

import (
	"encoding/json"
	"io"
	"net/http"
	"time"
)

type uploadResult struct {
	ID      string    `json:"upload_id"`
	Offset  int64     `json:"offset"`
	Size    int64     `json:"size"`
	State   string    `json:"state"`
	Expires time.Time `json:"expires_at"`
}

func uploadRequest(c *webClient, method, path string, body io.Reader, headers map[string]string) (int, uploadResult) {
	req, err := http.NewRequestWithContext(c.ctx, method, c.origin+"/web/v1/"+path, body)
	if err != nil {
		return 0, uploadResult{}
	}
	req.Header.Set("Origin", c.origin)
	req.Header.Set("Content-Type", "application/octet-stream")
	req.AddCookie(c.cookie)
	for name, value := range headers {
		req.Header.Set(name, value)
	}
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		return 0, uploadResult{}
	}
	defer response.Body.Close()
	var result uploadResult
	json.NewDecoder(response.Body).Decode(&result)
	return response.StatusCode, result
}
