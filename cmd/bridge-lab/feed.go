package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"flag"
	"io"
	"net"
	"net/http"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type feedRequest struct {
	Body       string                  `json:"body"`
	Method     string                  `json:"method"`
	Command    *contracts.Command      `json:"command"`
	Query      *contracts.CommandQuery `json:"query"`
	Delegation struct {
		Level       string   `json:"local_level"`
		Principal   string   `json:"local_principal"`
		Grants      []string `json:"local_grants"`
		CommandID   string   `json:"command_id"`
		RequestHash string   `json:"request_hash"`
	} `json:"delegation"`
}

func feedSend(ctx context.Context, args []string) error {
	set := flag.NewFlagSet("shadow-feed", flag.ContinueOnError)
	peerHome := set.String("peer-home", "", "synthetic replica home")
	address := set.String("address", "", "gateway peer listener")
	if err := set.Parse(args); err != nil {
		return err
	}
	var request feedRequest
	if err := json.NewDecoder(os.Stdin).Decode(&request); err != nil {
		return err
	}
	authority, err := trust.Open(*peerHome)
	if err != nil {
		return err
	}
	enrollment, err := authority.Enrollment()
	if err != nil || enrollment == nil {
		return contracts.ErrUnavailable
	}
	var body []byte
	hash, id := request.Delegation.RequestHash, request.Delegation.CommandID
	if request.Body != "" {
		if body, err = base64.StdEncoding.DecodeString(request.Body); err != nil {
			return err
		}
	} else {
		if request.Method != "submit" && request.Method != "status" && request.Method != "list" {
			return contracts.ErrInvalid
		}
		if body, err = marshalForwarded(request.Method, request.Command, request.Query); err != nil {
			return err
		}
	}
	if request.Method == "submit" && request.Command != nil {
		id = request.Command.ID
		if hash == "" {
			if hash, err = contracts.CommandHash(*request.Command); err != nil {
				return err
			}
		}
	} else if request.Query != nil && id == "" {
		id = request.Query.ID
	}
	if hash == "" {
		sum := sha256.Sum256(body)
		hash = hex.EncodeToString(sum[:])
	}
	grants := make([]contracts.Grant, len(request.Delegation.Grants))
	for i, grant := range request.Delegation.Grants {
		grants[i] = contracts.Grant(grant)
	}
	principal := trust.Principal{ID: request.Delegation.Principal, Grants: grants, Level: trust.AgentLevel(request.Delegation.Level)}
	if principal.Level != "" {
		principal.Kind = "agent"
	}
	header, err := authority.SignDelegation(principal, id, hash)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, "POST", "https://"+*address+"/v1/commands", bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("X-Apple-Bridge-Delegation", header)
	transport := &http.Transport{TLSClientConfig: authority.ClientTLS(enrollment.Fingerprint), DialContext: (&net.Dialer{Timeout: 2 * time.Second}).DialContext, TLSHandshakeTimeout: 5 * time.Second}
	defer transport.CloseIdleConnections()
	resp, err := (&http.Client{Transport: transport, Timeout: 60 * time.Second}).Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, contracts.MaxQueryFrame))
	if err != nil {
		return err
	}
	outcome := map[string]any{"status": 200}
	if resp.StatusCode != 200 {
		var envelope struct {
			Error *contracts.Error `json:"error"`
		}
		if json.Unmarshal(raw, &envelope) != nil || envelope.Error == nil {
			return contracts.ErrUnavailable
		}
		outcome = map[string]any{"status": 400, "error": envelope.Error.Code}
	} else if request.Method == "list" {
		var rows []json.RawMessage
		if err = json.Unmarshal(raw, &rows); err != nil {
			return err
		}
		outcome["count"] = len(rows)
	} else {
		var record struct {
			State       string `json:"state"`
			Disposition string `json:"disposition"`
		}
		if err = json.Unmarshal(raw, &record); err != nil {
			return err
		}
		outcome["state"], outcome["disposition"] = record.State, record.Disposition
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"outcome": outcome, "request_hash": hash, "command_id": id})
}

func marshalForwarded(method string, command *contracts.Command, query *contracts.CommandQuery) ([]byte, error) {
	value := struct {
		Method  string                 `json:"method"`
		Command contracts.Command      `json:"command"`
		Query   contracts.CommandQuery `json:"query"`
	}{Method: method}
	if command != nil {
		value.Command = *command
	}
	if query != nil {
		value.Query = *query
	}
	return json.Marshal(value)
}
