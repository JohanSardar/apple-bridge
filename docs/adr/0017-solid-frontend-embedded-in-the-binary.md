# SolidJS frontend embedded in the bridge binary

The UI must hit an 8 ms interaction budget and scroll 100k-message threads at 120 fps. SolidJS updates the DOM with fine-grained reactivity and no virtual DOM, and it brings a tiny runtime. Everything else is ours: springs, virtual list, router. The built assets are embedded with `go:embed`, so users run one binary and never need Node; contributors need pnpm only to change the UI.
