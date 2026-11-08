# Eilmer public website

Buildless static HTML and CSS. No dependencies, accounts, analytics or payment flow.

Stage into a new directory from the repository root:

```sh
python3 packaging/site/package.py /tmp/eilmer-site
```

The staging command copies `docs/install` verbatim to `applebridge/install`;
that file remains the installer source of truth for both the new endpoint and
legacy GitHub Pages. Never edit a second installer copy here.

Serve the staged directory on `eilmer.si` only. Serve `/` as `index.html`,
`/applebridge` and `/applebridge/` as `applebridge/index.html`, `/assets/site.css`
as `text/css`, and `/applebridge/install` as `text/plain; charset=utf-8` with no
HTML fallback. Return a real 404 for missing files. Keep the existing relay
hostname and customer-host routing separate. T2 owns host configuration and
atomic deployment/rollback; this directory does not change the relay runtime.

The staging command excludes this README. Publish the two HTML pages, CSS,
and staged installer. Verify HTTP status, content type and installer bytes
on both the canonical URL and the retained GitHub Pages URL before launch.
