package trust

import "time"

type OAuthClient struct {
	ID                      string   `json:"client_id"`
	Name                    string   `json:"client_name"`
	RedirectURIs            []string `json:"redirect_uris"`
	TokenEndpointAuthMethod string   `json:"token_endpoint_auth_method"`
	GrantTypes              []string `json:"grant_types"`
	ResponseTypes           []string `json:"response_types"`
}
type OAuthCode struct {
	Hash        string    `json:"hash"`
	ClientID    string    `json:"client_id"`
	RedirectURI string    `json:"redirect_uri"`
	Challenge   string    `json:"challenge"`
	Resource    string    `json:"resource"`
	Principal   string    `json:"principal"`
	ExpiresAt   time.Time `json:"expires_at"`
	Used        bool      `json:"used"`
}
type OAuthAccess struct {
	Hash      string    `json:"hash"`
	Principal string    `json:"principal"`
	Resource  string    `json:"resource"`
	ExpiresAt time.Time `json:"expires_at"`
}
type OAuthRefresh struct {
	Hash      string    `json:"hash"`
	ClientID  string    `json:"client_id"`
	Resource  string    `json:"resource"`
	Principal string    `json:"principal"`
	ExpiresAt time.Time `json:"expires_at"`
	Used      bool      `json:"used"`
}
type OAuthTokens struct {
	AccessToken  string `json:"access_token"`
	TokenType    string `json:"token_type"`
	ExpiresIn    int    `json:"expires_in"`
	RefreshToken string `json:"refresh_token"`
	Scope        string `json:"scope"`
}
