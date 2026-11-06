# Apple Bridge

Connects agents on your Mac, such as Claude Code and Codex, to your Messages, Contacts, Calendar, Reminders, Notes and Mail. It needs a Mac with Apple silicon running macOS 26 or later.

This package installs the `bridge` command and the signed Apple Bridge app that holds your Mac permissions. It has no install script, and it downloads nothing during install.

```sh
bridge up                   # start Apple Bridge. It asks nothing
bridge link claude          # connect Claude Code. Or: bridge link codex
bridge permissions guide    # allow the Mac permissions you want
bridge stats --watch        # counts per source, as each one finishes
```

## Updating

Stop the gateway first, so the running version and the installed one don't differ:

```sh
bridge down
npm update -g PACKAGE_NAME
bridge up
```

Your Mac permissions belong to the signing identity and app identifier, and both stay the same between releases.

## Uninstalling

Stop Apple Bridge and disconnect your agents first. Removing the package does not stop a gateway that is already running, and it removes the commands that stop it:

```sh
bridge unlink claude        # and: bridge unlink codex, for the agents you connected
bridge down
npm uninstall -g PACKAGE_NAME
```

If `bridge down` says it did not start the gateway, the Apple Bridge app is running it. Use the app's own uninstall instead, or quit it first.

If you already uninstalled while it was running, install the package again, then run `bridge down` and uninstall.

These stay on your Mac, in case you install again or also use the app:

- Your data, in `~/Library/Application Support/AppleBridge`. It holds a copy of your private messages and mail, plus Apple Bridge's own keychain file with its credentials. Delete this folder when you no longer want them.
- Any cached files, in `~/Library/Caches/AppleBridge`. You can delete this folder.
- Mac permissions, listed in System Settings under Privacy & Security. Turn them off there.

## The app

The app is inside this package, at `Apple Bridge.app`. `bridge` finds it on its own. `bridge up` does not start again after a restart; run it again, or use the Apple Bridge app from the project page, which starts at login and updates itself. If that app is already running, `bridge up` reports it and leaves it alone.

Documentation: <https://github.com/JohanSardar/apple-bridge>
