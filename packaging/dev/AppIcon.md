# App icon

`AppIcon.png` is the owner's icon from [apple-bridge-iphone](https://github.com/JohanSardar/apple-bridge-iphone), `ios/App/Assets.xcassets/AppIcon.appiconset/AppIcon.png`, Git blob `2e416cab2eb046d24816556cc68c2609995d405e`. It is reused unchanged, not third-party artwork.

`AppIcon.icns` was generated on macOS by resizing that 1024 × 1024 PNG with `sips` into an `AppIcon.iconset` with 16, 32, 128, 256 and 512 pixel entries and their `@2x` counterparts, then running:

```sh
iconutil -c icns AppIcon.iconset -o AppIcon.icns
```

Both app and development Helper builds copy it to `Contents/Resources/AppIcon.icns` before signing. `Info.plist` declares `CFBundleIconFile`, and the signed Helper cache includes the icon bytes in its input key.
