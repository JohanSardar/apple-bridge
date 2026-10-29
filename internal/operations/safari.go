package operations

import (
	"context"
	"encoding/json"
	"io"
	"mime"
	"net/http"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/publichttp"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"golang.org/x/net/html"
)

func FetchSafariPage(ctx context.Context, guard *publichttp.Guard, raw string) (json.RawMessage, error) {
	u, err := publichttp.URL(raw)
	if err != nil {
		return nil, err
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	if _, err = guard.Addresses(ctx, u.Hostname()); err != nil {
		return nil, err
	}
	client, close := guard.Client()
	defer close()
	req, err := http.NewRequestWithContext(ctx, "GET", u.String(), nil)
	if err != nil {
		return nil, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, contracts.ErrInvalid
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, contracts.ErrUnavailable
	}
	kind, _, _ := mime.ParseMediaType(resp.Header.Get("Content-Type"))
	if kind != "text/html" && kind != "text/plain" {
		return nil, contracts.ErrUnsupported
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, 262145))
	if err != nil {
		return nil, err
	}
	if len(data) > 262144 {
		return nil, contracts.ErrCapacity
	}
	if !utf8.Valid(data) {
		return nil, contracts.ErrUnsupported
	}
	text := string(data)
	if kind == "text/html" {
		z := html.NewTokenizer(strings.NewReader(text))
		var parts []string
		skip := 0
		for {
			tt := z.Next()
			if tt == html.ErrorToken {
				if z.Err() != io.EOF {
					return nil, contracts.ErrCorrupt
				}
				break
			}
			token := z.Token()
			switch tt {
			case html.StartTagToken:
				if token.Data == "script" || token.Data == "style" || token.Data == "noscript" {
					skip++
				}
			case html.EndTagToken:
				if (token.Data == "script" || token.Data == "style" || token.Data == "noscript") && skip > 0 {
					skip--
				}
			case html.TextToken:
				if skip == 0 {
					parts = append(parts, token.Data)
				}
			}
		}
		text = strings.Join(parts, " ")
	}
	text = strings.Join(strings.Fields(text), " ")
	return json.Marshal(map[string]any{"text": text, "url": resp.Request.URL.String(), "limited": "logged-in pages unavailable; no Safari cookies are used"})
}
func (g *LocalGateway) safariPage(ctx context.Context, p trust.Principal, q contracts.Query) (contracts.QueryResult, error) {
	var a struct {
		Tab string `json:"tab"`
	}
	if contracts.DecodeInput(q.Args, &a) != nil || a.Tab == "" {
		return contracts.QueryResult{}, contracts.ErrInvalid
	}
	if g.State == nil {
		return contracts.QueryResult{}, contracts.ErrUnavailable
	}
	rows, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "safari.tab", ID: a.Tab})
	if err != nil {
		return contracts.QueryResult{}, err
	}
	if len(rows.Records) != 1 || rows.Records[0].Deleted {
		return contracts.QueryResult{}, contracts.ErrNotFound
	}
	var data struct {
		URL string `json:"url"`
	}
	if json.Unmarshal(rows.Records[0].Data, &data) != nil {
		return contracts.QueryResult{}, contracts.ErrCorrupt
	}
	raw, err := FetchSafariPage(ctx, &publichttp.Guard{}, data.URL)
	return contracts.QueryResult{ID: q.ID, ResultJSON: raw, Blobs: []contracts.QueryBlob{}}, err
}
