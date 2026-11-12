# Eilmer public website

Buildless static HTML and CSS. No dependencies, accounts, analytics or payment flow.

Stage into a new directory from the repository root:

```sh
python3 packaging/site/package.py /tmp/eilmer-site
```

The staging command copies `docs/install` verbatim to `applebridge/install`;
that file remains the installer source of truth. Never edit a second
installer copy here. It also copies `deploy/selfhost/install.sh` to
`applebridge/selfhost/install.sh`.

Serve the staged directory on `eilmer.si` only. Serve `/` as `index.html`,
`/applebridge` and `/applebridge/` as `applebridge/index.html`, `/assets/site.css`
as `text/css`, and `/applebridge/install` as `text/plain; charset=utf-8` with no
HTML fallback. Return a real 404 for missing files. Keep the existing relay
hostname and customer-host routing separate. T2 owns host configuration and
atomic deployment/rollback; this directory does not change the relay runtime.

The staging command excludes this README. Publish the two HTML pages, CSS,
and staged installer. Verify HTTP status, content type and installer bytes
on the public URL before launch. Release files are not in git and not in the staged
site: they live in a separate folder on the server, passed to `deploy/relay/site.py`
as `--downloads`, so redeploying the site never removes them. They are served at
`/applebridge/download/<tag>/<file>`. Relay bundles go in `relay-<version>/` as
`ab-relay-<version>-linux-amd64.tar.gz` plus its `.sha256`, and the file
`relay-latest` holds the newest tag, such as `relay-0.1.7`, which
`deploy/selfhost/install.sh` downloads by default. No relay release has been
published yet.
