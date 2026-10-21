package replication

import (
	"context"
	"crypto/rand"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"errors"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
)

const delegationProtocol = "delegation/1"

type delegationRequest struct {
	Principal   string `json:"principal"`
	CommandID   string `json:"command_id"`
	RequestHash string `json:"request_hash"`
	Challenge   string `json:"challenge"`
}

type delegationSession struct {
	commandSession
	certificate *x509.Certificate
}

func (s *Server) registerDelegation(ctx context.Context, conn *websocket.Conn, p trust.Principal, cert *x509.Certificate) func() {
	session := &delegationSession{commandSession: commandSession{conn: conn, ctx: ctx, pending: map[string]chan contracts.CommandResponse{}}, certificate: cert}
	s.mu.Lock()
	if s.delegations == nil {
		s.delegations = map[string]*delegationSession{}
	}
	s.delegations[p.ID] = session
	s.mu.Unlock()
	return func() {
		s.mu.Lock()
		if s.delegations[p.ID] == session {
			delete(s.delegations, p.ID)
		}
		s.mu.Unlock()
	}
}
func (s *Server) deliverDelegation(id string, conn *websocket.Conn, f contracts.FeedFrame) bool {
	s.mu.Lock()
	session := s.delegations[id]
	s.mu.Unlock()
	if session == nil || session.conn != conn || f.Response == nil || f.ID == "" {
		return false
	}
	session.deliver(f.ID, *f.Response)
	return true
}
func (s *Server) Reauthorize(ctx context.Context, principal, id, hash string) (trust.Principal, error) {
	empty := trust.Principal{}
	parent, local, ok := strings.Cut(principal, "/")
	if !ok || !trust.IsAgentPrincipalID(principal) {
		return empty, contracts.ErrUnauthorized
	}
	paired, err := s.Authority.Principal(parent)
	if err != nil || paired.Kind != "replica" {
		return empty, contracts.ErrUnauthorized
	}
	s.mu.Lock()
	session := s.delegations[parent]
	s.mu.Unlock()
	if session == nil {
		return empty, contracts.ErrUnavailable
	}
	nonce := make([]byte, 32)
	if _, err := rand.Read(nonce); err != nil {
		return empty, contracts.ErrUnavailable
	}
	challenge := hex.EncodeToString(nonce)
	request, _ := json.Marshal(delegationRequest{local, id, hash, challenge})
	session.mu.Lock()
	if len(session.pending) >= 16 {
		session.mu.Unlock()
		return empty, contracts.ErrUnavailable
	}
	requestID := contracts.NewID("delegation")
	response := make(chan contracts.CommandResponse, 1)
	session.pending[requestID] = response
	session.mu.Unlock()
	defer func() { session.mu.Lock(); delete(session.pending, requestID); session.mu.Unlock() }()
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	if _, err = send(bounded, session.conn, contracts.FeedFrame{T: "DelegationRequest", ID: requestID, Request: request}); err != nil {
		return empty, contracts.ErrUnavailable
	}
	select {
	case reply := <-response:
		if reply.Error != nil {
			return empty, reply.Error
		}
		var proof string
		if json.Unmarshal(reply.Result, &proof) != nil {
			return empty, contracts.ErrUnauthorized
		}
		actor, err := s.Authority.VerifyDelegationChallenge(session.certificate, proof, id, hash, challenge)
		if err != nil || actor.ID != principal || !actor.IsAgent() {
			return empty, contracts.ErrUnauthorized
		}
		return actor, nil
	case <-session.ctx.Done():
		return empty, contracts.ErrUnavailable
	case <-bounded.Done():
		return empty, contracts.ErrUnavailable
	}
}
func (c *Client) answerDelegation(ctx context.Context, conn *websocket.Conn, frame contracts.FeedFrame) error {
	var request delegationRequest
	reply := contracts.CommandResponse{Error: contracts.ErrUnauthorized}
	if frame.ID == "" || len(frame.Request) > 4096 || contracts.DecodeInput(frame.Request, &request) != nil || !trust.IsAgentPrincipalID(request.Principal) || strings.Contains(request.Principal, "/") || !contracts.ValidCommandID(request.CommandID) || !contracts.ValidDigest(request.RequestHash) || !contracts.ValidDigest(request.Challenge) {
		return contracts.ErrInvalid
	}
	principal, err := c.Authority.Principal(request.Principal)
	if err == nil && principal.Kind == "agent" {
		enrollment, loadErr := c.Authority.Enrollment()
		if loadErr != nil || enrollment == nil {
			return contracts.ErrUnavailable
		}
		principal.Grants = trust.EffectiveGrants(principal.Grants, enrollment.Grants)
		proof, signErr := c.Authority.SignDelegationChallenge(principal, request.CommandID, request.RequestHash, request.Challenge)
		if signErr == nil {
			reply.Result, err = json.Marshal(proof)
			reply.Error = nil
		} else {
			reply.Error = contracts.ErrUnavailable
		}
	} else if err != nil && !errors.Is(err, contracts.ErrUnauthorized) {
		reply.Error = contracts.ErrUnavailable
	}
	_, err = send(ctx, conn, contracts.FeedFrame{T: "DelegationResponse", ID: frame.ID, Response: &reply})
	return err
}
