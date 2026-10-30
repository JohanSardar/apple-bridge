package main

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"flag"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"math/big"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/frontdoor"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func main() {
	home := flag.String("home", "", "isolated test home")
	address := flag.String("address", "https://100.78.200.121:17781", "phone-visible HTTPS address")
	listen := flag.String("listen", ":17781", "TLS test listener")
	flag.Parse()
	if *home == "" {
		panic("isolated test home required")
	}
	a, err := trust.Open(*home)
	if err != nil {
		panic(err)
	}
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		panic(err)
	}
	template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "ab-test-iphone"}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		panic(err)
	}
	certificate := tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
	a.Fingerprint = trust.Fingerprint(der)
	code, err := a.PairIPhoneCode(*address, false, 10*time.Minute)
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile(filepath.Join(*home, "pair-code"), []byte(code), 0600); err != nil {
		panic(err)
	}
	endpoint, err := url.Parse(*address)
	if err != nil {
		panic(err)
	}
	store, err := state.Open(filepath.Join(*home, "state"))
	if err != nil {
		panic(err)
	}
	defer store.Close()
	if err = store.InitCommands(context.Background()); err != nil {
		panic(err)
	}
	blobStore, err := blobs.Open(*home, blobs.Config{})
	if err != nil {
		panic(err)
	}
	delivery := iphone.NewDelivery(store, a)
	delivery.Blobs = blobStore
	gateway := &operations.LocalGateway{State: store, Authority: a, Context: context.Background(), IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
	delivery.Decide = gateway.DecidePhoneApproval
	if err = gateway.Recover(context.Background()); err != nil {
		panic(err)
	}
	go delivery.Run(context.Background())
	token, err := a.CreateAPIToken([]contracts.Grant{"iphone:write", "iphone:read"}, true)
	if err != nil {
		panic(err)
	}
	if err = os.WriteFile(filepath.Join(*home, "api-token"), []byte(token), 0600); err != nil {
		panic(err)
	}
	mux := frontdoor.Handler(frontdoor.Deps{IPhone: delivery, Authority: a, Operations: &operations.Service{State: store, Target: gateway, Authority: a}, PublicHost: func() string { return endpoint.Host }}, contracts.Operations)
	server := &http.Server{Addr: *listen, Handler: mux, TLSConfig: &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{certificate}}, ReadHeaderTimeout: 10 * time.Second}
	json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true})
	if err = server.ListenAndServeTLS("", ""); err != nil {
		panic(err)
	}
}
