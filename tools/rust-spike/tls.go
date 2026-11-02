package main

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"fmt"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func tlsSpike(args []string) error {
	dir := args[1]
	if args[0] == "generate" {
		for _, name := range []string{"go", "rust", "go-expired", "rust-expired"} {
			pub, key, e := ed25519.GenerateKey(rand.Reader)
			if e != nil {
				return e
			}
			template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "synthetic"}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth}}
			if len(name) > 4 {
				template.NotBefore = time.Now().Add(-2 * time.Hour)
				template.NotAfter = time.Now().Add(-time.Hour)
			}
			der, e := x509.CreateCertificate(rand.Reader, template, template, pub, key)
			if e != nil {
				return e
			}
			raw, e := x509.MarshalPKCS8PrivateKey(key)
			if e != nil {
				return e
			}
			if e = os.WriteFile(filepath.Join(dir, name+".der"), der, 0600); e != nil {
				return e
			}
			if e = os.WriteFile(filepath.Join(dir, name+".key"), raw, 0600); e != nil {
				return e
			}
			fmt.Println(name, trust.Fingerprint(der))
		}
		return nil
	}
	cert, e := os.ReadFile(filepath.Join(dir, "go.der"))
	if e != nil {
		return e
	}
	raw, e := os.ReadFile(filepath.Join(dir, "go.key"))
	if e != nil {
		return e
	}
	key, e := x509.ParsePKCS8PrivateKey(raw)
	if e != nil {
		return e
	}
	a := &trust.Authority{Certificate: tls.Certificate{Certificate: [][]byte{cert}, PrivateKey: key}}
	if args[0] == "server" {
		listener, e := a.PinnedListener("127.0.0.1:0", args[2])
		if e != nil {
			return e
		}
		defer listener.Close()
		fmt.Println(listener.Addr().(*net.TCPAddr).Port)
		conn, e := listener.Accept()
		if e != nil {
			return e
		}
		defer conn.Close()
		conn.SetDeadline(time.Now().Add(10 * time.Second))
		b := make([]byte, 1)
		if _, e = conn.Read(b); e != nil {
			return e
		}
		_, e = conn.Write(b)
		return e
	}
	conn, e := tls.Dial("tcp", "127.0.0.1:"+args[3], a.ClientTLS(args[2]))
	if e != nil {
		return e
	}
	defer conn.Close()
	conn.SetDeadline(time.Now().Add(10 * time.Second))
	if _, e = conn.Write([]byte{'x'}); e != nil {
		return e
	}
	b := make([]byte, 1)
	_, e = conn.Read(b)
	return e
}
