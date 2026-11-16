# Eilmer public website

Buildless static HTML and CSS. No dependencies, accounts, analytics or payment flow.

Stage into a new directory from the repository root:

```sh
python3 packaging/site/package.py /tmp/eilmer-site
```

The staging command copies `docs/install` verbatim to `applebridge/install`;
that file remains the installer source of truth. Never edit a second
installer copy here. It excludes this README and `serve.py`.

Serve the staged directory on `eilmer.si` only with `deploy/site/serve.py`:

```sh
python3 deploy/site/serve.py /tmp/eilmer-site --downloads /srv/applebridge-downloads --port 8793
```

It listens on loopback and rejects HTTP Host values other than `eilmer.si`.
It serves `/` as `index.html`, `/applebridge` and `/applebridge/` as
`applebridge/index.html`, `/assets/site.css` as `text/css`, and
`/applebridge/install` as `text/plain; charset=utf-8` with no HTML fallback.
Every other path returns a real 404.

Release files are not in git and not in the staged site: they live in a
separate folder on the server, passed as `--downloads`, so redeploying the site
never removes them. They are served at `/applebridge/download/<tag>/<file>`.
Verify HTTP status, content type and installer bytes on the public URL before
launch.
