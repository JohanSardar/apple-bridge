package scenarios

import (
	"context"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"golang.org/x/net/dns/dnsmessage"
)

type acmeDNS struct {
	mu     sync.Mutex
	values map[string]string
	host   string
}

func (d *acmeDNS) TXT(_ context.Context, name, value string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.values[name+"."] = value
	return nil
}
func relayPebble(ctx context.Context, lab *relayLab, key ed25519.PrivateKey, credential relay.Credential, custom bool, add func(string, bool)) error {
	binary, err := pebbleBinary(ctx)
	if err != nil {
		return err
	}
	dir, err := os.MkdirTemp("", "ab-pebble-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)
	home := filepath.Join(dir, "bridge")
	os.MkdirAll(filepath.Join(home, "remote"), 0700)
	dns := &acmeDNS{values: map[string]string{}, host: credential.Hostname}
	lab.server.DNS = dns
	packet, err := net.ListenPacket("udp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	defer packet.Close()
	go func() {
		b := make([]byte, 4096)
		for {
			n, addr, err := packet.ReadFrom(b)
			if err != nil {
				return
			}
			var message dnsmessage.Message
			if message.Unpack(b[:n]) != nil {
				continue
			}
			message.Response = true
			message.Authoritative = true
			for _, q := range message.Questions {
				if q.Type == dnsmessage.TypeTXT {
					name := q.Name.String()
					if custom && name == "_acme-challenge.mac.example.test." {
						name = "_acme-challenge." + dns.host + "."
					}
					dns.mu.Lock()
					value := dns.values[name]
					dns.mu.Unlock()
					if value != "" {
						message.Answers = append(message.Answers, dnsmessage.Resource{Header: dnsmessage.ResourceHeader{Name: q.Name, Type: dnsmessage.TypeTXT, Class: dnsmessage.ClassINET, TTL: 1}, Body: &dnsmessage.TXTResource{TXT: []string{value}}})
					}
				}
			}
			raw, e := message.Pack()
			if e == nil {
				packet.WriteTo(raw, addr)
			}
		}
	}()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return err
	}
	address := listener.Addr().String()
	listener.Close()
	private, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return err
	}
	template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "Pebble lab"}, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().Add(time.Hour), IsCA: true, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageCertSign | x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	der, err := x509.CreateCertificate(rand.Reader, template, template, private.Public(), private)
	if err != nil {
		return err
	}
	rawKey, _ := x509.MarshalECPrivateKey(private)
	certPath, keyPath := filepath.Join(dir, "server.pem"), filepath.Join(dir, "server-key.pem")
	os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0600)
	os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: rawKey}), 0600)
	config := map[string]any{"pebble": map[string]any{"listenAddress": address, "managementListenAddress": "127.0.0.1:0", "certificate": certPath, "privateKey": keyPath, "httpPort": 5002, "tlsPort": 5001, "profiles": map[string]any{"default": map[string]any{"description": "lab renewal", "validityPeriod": 86400}}, "retryAfter": map[string]int{"authz": 1, "order": 1}}}
	raw, _ := json.Marshal(config)
	configPath := filepath.Join(dir, "pebble.json")
	os.WriteFile(configPath, raw, 0600)
	command := exec.CommandContext(ctx, binary, "-config", configPath, "-dnsserver", packet.LocalAddr().String())
	command.Env = append(os.Environ(), "PEBBLE_VA_NOSLEEP=1", "PEBBLE_WFE_NONCEREJECT=0")
	log, err := os.Create(filepath.Join(dir, "pebble.log"))
	if err != nil {
		return err
	}
	defer log.Close()
	command.Stdout = log
	command.Stderr = log
	if err = command.Start(); err != nil {
		return err
	}
	defer func() { command.Process.Kill(); command.Wait() }()
	api := httptest.NewTLSServer(lab.server.Handler(""))
	defer api.Close()
	transport := api.Client().Transport.(*http.Transport).Clone()
	defer transport.CloseIdleConnections()
	root, _ := x509.ParseCertificate(der)
	transport.TLSClientConfig.RootCAs.AddCert(root)
	client := &http.Client{Transport: transport, Timeout: 10 * time.Second}
	directory := "https://" + address + "/dir"
	for deadline := time.Now().Add(5 * time.Second); ; {
		resp, e := client.Get(directory)
		if e == nil {
			resp.Body.Close()
			break
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("Pebble did not start")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
	host := credential.Hostname
	if custom {
		host = "mac.example.test"
	}
	options := remote.CertificateOptions{HTTPClient: client, DirectoryURL: directory}
	bounded, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	if err = remote.ProvisionCertificate(bounded, home, host, api.URL, key, credential, options); err != nil {
		return fmt.Errorf("issue: %w", err)
	}
	certificatePath := filepath.Join(home, "remote", "relay-cert.pem")
	first, _ := os.ReadFile(certificatePath)
	block, _ := pem.Decode(first)
	if block == nil {
		return fmt.Errorf("certificate missing")
	}
	leaf, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return err
	}
	add("pebble_certificate_hostname", leaf.VerifyHostname(host) == nil)
	if err = remote.ProvisionCertificate(bounded, home, host, api.URL, key, credential, options); err != nil {
		return fmt.Errorf("renew: %w", err)
	}
	second, _ := os.ReadFile(certificatePath)
	block, _ = pem.Decode(second)
	renewed, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return err
	}
	add("renewal_inside_30_days", renewed.SerialNumber.Cmp(leaf.SerialNumber) != 0)
	info, err := os.Stat(filepath.Join(home, "remote", "relay-key.pem"))
	add("private_key_only_under_bridge_home", err == nil && info.Mode().Perm() == 0600)
	dns.mu.Lock()
	value := dns.values["_acme-challenge."+credential.Hostname+"."]
	dns.mu.Unlock()
	add("dns_challenge_cleaned_up", value == "")
	if !custom {
		control, e := remote.ControlTLSConfig(bounded, filepath.Join(dir, "control"), host, dns, options)
		add("control_dns01_certificate_issued", e == nil)
		if e != nil {
			return e
		}
		certificate, e := control.GetCertificate(nil)
		add("control_certificate_available", e == nil && certificate != nil)
		if e == nil && certificate != nil {
			leaf, e := x509.ParseCertificate(certificate.Certificate[0])
			add("control_certificate_hostname", e == nil && leaf.VerifyHostname(host) == nil)
		}
	}

	return nil
}

func pebbleBinary(ctx context.Context) (string, error) {
	if binary := os.Getenv("AB_LAB_PEBBLE"); binary != "" {
		return binary, nil
	}
	cache, err := os.UserCacheDir()
	if err != nil {
		return "", err
	}
	bin := filepath.Join(cache, "apple-bridge", "lab-tools")
	binary := filepath.Join(bin, "pebble")
	if _, err := os.Stat(binary); err == nil {
		return binary, nil
	}
	if err := os.MkdirAll(bin, 0700); err != nil {
		return "", err
	}
	install := exec.CommandContext(ctx, "go", "install", "github.com/letsencrypt/pebble/v2/cmd/pebble@v2.8.0")
	install.Env = append(os.Environ(), "GOBIN="+bin, "GOFLAGS=-mod=mod")
	if output, err := install.CombinedOutput(); err != nil {
		return "", fmt.Errorf("install pebble: %w: %s", err, output)
	}
	return binary, nil
}
