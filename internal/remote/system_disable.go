package remote

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"tailscale.com/client/local"
	"tailscale.com/ipn"
	"time"
)

func removeSystem(ctx context.Context, home string, client *local.Client) error {
	var entry lease
	if err := read(home, "lease", &entry); os.IsNotExist(err) {
		return nil
	} else if err != nil {
		return err
	}
	hp := net.JoinHostPort(entry.Host, strconv.Itoa(int(entry.Port)))
	for attempt := 0; attempt < 5; attempt++ {
		config, etag, err := getServe(ctx, client)
		if err != nil {
			return err
		}
		web, err := section(config, "Web")
		if err != nil {
			return err
		}
		expected := raw(ipn.WebServerConfig{Handlers: map[string]*ipn.HTTPHandler{"/": {Proxy: entry.Target}}})
		if _, exists := web[hp]; !exists {
			return os.Remove(filepath.Join(home, "remote", "lease.json"))
		}
		var compact bytes.Buffer
		if json.Compact(&compact, web[hp]) != nil || !bytes.Equal(compact.Bytes(), expected) {
			return fmt.Errorf("our Funnel mapping was changed; refusing to remove it")
		}
		for _, key := range []string{"TCP", "Web", "AllowFunnel"} {
			values, err := section(config, key)
			if err != nil {
				return err
			}
			index := hp
			if key == "TCP" {
				index = strconv.Itoa(int(entry.Port))
				var tcp bytes.Buffer
				if json.Compact(&tcp, values[index]) != nil || !bytes.Equal(tcp.Bytes(), raw(ipn.TCPPortHandler{HTTPS: true})) {
					return fmt.Errorf("our TCP mapping was changed; refusing to remove it")
				}
			}
			delete(values, index)
			if len(values) == 0 && entry.Absent[key] {
				delete(config, key)
			} else {
				config[key] = raw(values)
			}
		}
		ok, err := setServe(ctx, client, config, etag)
		if err != nil {
			return err
		}
		if ok {
			return os.Remove(filepath.Join(home, "remote", "lease.json"))
		}
	}
	return fmt.Errorf("serve config changed during five ETag attempts")
}

func cleanupSystem(home string, client *local.Client) error {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	return removeSystem(ctx, home, client)
}
