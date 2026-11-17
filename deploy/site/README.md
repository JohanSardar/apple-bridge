# Eilmer public website

Buildless static HTML and CSS. No dependencies, accounts, analytics or payment flow.
The site serves `https://eilmer.si`, the Apple Bridge page and the curl install.
Nothing else runs for Apple Bridge on the server.

Stage into a new directory from the repository root:

```sh
python3 packaging/site/package.py /tmp/eilmer-site
```

The staging command copies `docs/install` verbatim to `applebridge/install`;
that file remains the installer source of truth. Never edit a second
installer copy here. It excludes this README, `serve.py`, `Caddyfile` and
`eilmer-site.service`.

## What serves it

`serve.py` serves the staged directory on `eilmer.si` only:

```sh
python3 deploy/site/serve.py /tmp/eilmer-site --downloads /srv/eilmer/downloads --port 8793
```

It listens on loopback and rejects HTTP Host values other than `eilmer.si`.
It serves `/` as `index.html`, `/applebridge` and `/applebridge/` as
`applebridge/index.html`, `/assets/site.css` as `text/css`, and
`/applebridge/install` as `text/plain; charset=utf-8` with no HTML fallback.
The version-matched agent instructions are served as plain text at `/applebridge/setup.md`; Markdown documentation is served at `/applebridge/docs/<path>.md`. The package includes the same setup instructions that `bridge skills get setup` prints. Unknown paths, non-Markdown docs and traversal outside that directory return a real 404.

Release files are not in git and not in the staged site: they live in a
separate folder on the server, passed as `--downloads`, so redeploying the site
never removes them. They are served at `/applebridge/download/<tag>/<file>`.

## Live server

The site runs on a Debian 12 VM whose static address is the `eilmer.si` A
record. Only TCP 443 is open to the internet.

| Path | Contents |
| --- | --- |
| `/opt/eilmer-site/serve.py` | `serve.py` from this folder |
| `/srv/eilmer/site` | output of `packaging/site/package.py` |
| `/srv/eilmer/downloads/v<version>/` | release archives |
| `/etc/systemd/system/eilmer-site.service` | `eilmer-site.service` from this folder |
| `/etc/caddy/Caddyfile` | `Caddyfile` from this folder |

Everything under `/opt/eilmer-site` and `/srv/eilmer` is `root:root`, files
mode 644. `eilmer-site.service` runs `serve.py` on `127.0.0.1:8793` as a
systemd dynamic user that can only read.

Caddy (the Debian `caddy` package) owns port 443. It terminates TLS for
`eilmer.si`, proxies to `serve.py` and renews its Let's Encrypt certificate by
itself with the TLS-ALPN challenge on 443, so port 80 stays closed. Any other
name gets a TLS error.

First setup on a new server:

```sh
sudo apt-get install caddy
sudo install -m 644 Caddyfile /etc/caddy/Caddyfile
sudo install -m 644 eilmer-site.service /etc/systemd/system/
sudo install -D -m 644 serve.py /opt/eilmer-site/serve.py
sudo systemctl daemon-reload
sudo systemctl enable --now eilmer-site caddy
sudo systemctl reload caddy
```

The package starts Caddy with its default config during install, so the
reload is what applies this `Caddyfile`.

## Redeploying

1. Upload a release archive to `/srv/eilmer/downloads/v<version>/`
   (`root:root`, 644) before pinning it in `docs/install`.
2. After the pin merges, stage the site from a clean checkout of `main`, copy
   it to the server, and replace `/srv/eilmer/site` by renaming the old folder
   aside and moving the new one in. Keep the old folder until the checks below
   pass.
3. If `serve.py` changed, install it to `/opt/eilmer-site/serve.py` and run
   `sudo systemctl restart eilmer-site`. If the `Caddyfile` changed, install it
   and run `sudo systemctl reload caddy`.

Then check the public URLs: the installer at
`https://eilmer.si/applebridge/install` must be byte for byte `docs/install`
with `text/plain; charset=utf-8`, the pinned archive must download with the
pinned SHA-256, and `curl -fsSL https://eilmer.si/applebridge/install | sh`
with a throwaway `HOME` must install. The installer writes only under `HOME`.
