package main

import (
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
)

func main() {
	var v struct {
		Home     string            `json:"home"`
		Op       string            `json:"op"`
		Client   trust.OAuthClient `json:"client"`
		Code     trust.OAuthCode   `json:"code"`
		Token    string            `json:"token"`
		Exchange map[string]string `json:"exchange"`
	}
	if err := json.NewDecoder(os.Stdin).Decode(&v); err != nil {
		panic(err)
	}
	a, err := trust.Open(v.Home)
	var out any
	if err == nil {
		switch v.Op {
		case "register":
			out, err = a.RegisterOAuth(v.Client)
		case "issue":
			var agent trust.Agent
			agent, _, err = a.CreateAgent("synthetic", trust.AgentApprove, []contracts.Grant{"messages:write"})
			if err == nil {
				v.Code.Principal = agent.ID
				var raw string
				raw, err = a.IssueOAuthCode(v.Code)
				out = map[string]any{"code": raw, "principal": agent.ID}
			}
		case "exchange":
			e := v.Exchange
			out, err = a.ExchangeOAuth(e["grant_type"], e["client_id"], e["code"], e["redirect_uri"], e["code_verifier"], e["resource"], e["refresh_token"], e["scope"])
		case "authenticate":
			out, err = a.AuthenticateToken(v.Token)
		default:
			err = fmt.Errorf("op")
		}
	}
	if err != nil {
		out = map[string]string{"error": err.Error()}
	}
	if err = json.NewEncoder(os.Stdout).Encode(out); err != nil {
		panic(err)
	}
}
