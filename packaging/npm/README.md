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

Your Mac permissions belong to the signing identity and app identifier, and both stay the same between releases. If you update without stopping, `bridge up` warns when the running executable differs from the installed one. Run `bridge down`, then `bridge up` to finish. A disconnected Helper can leave sources partly available: stored records remain readable, but new changes may not sync until the restart. Restart the app instead if it owns the gateway.

## Uninstalling

Run `bridge down` before uninstalling, and disconnect your agents. Removing the package removes the commands that stop it. The gateway does not stop automatically when its installation is moved, replaced or deleted:

```sh
bridge unlink claude        # and: bridge unlink codex, for the agents you connected
bridge down
npm uninstall -g PACKAGE_NAME
```

If `bridge down` says it did not start the gateway, the Apple Bridge app is running it. Use the app's own uninstall instead, or quit it first.

If you already uninstalled while it was running, install the package again, then run `bridge down` and uninstall.

These stay on your Mac, in case you install again or also use the app:

- Your data, in `~/Library/Application Support/AppleBridge`. It holds a copy of your private messages and mail, plus Apple Bridge's own keychain file with its credentials. Delete this folder when you no longer want them.
- Permission-guide reports, in `~/Library/Application Support/AppleBridgeDev`. They can contain permission status and probe results. Delete this folder when no guide or development probe is running.
- Any cached files, in `~/Library/Caches/AppleBridge`. You can delete this folder.
- Mac permissions, listed in System Settings under Privacy & Security. Turn them off there.

A gateway started by `bridge up` checks its own `gateway.log` each second. Above 8 MiB it rotates the file to `gateway.log.1`, replacing the previous archive, and redirects its stdout and stderr to a new log. Nothing truncates an active writer. One archive is retained rather than unlimited history; a burst can exceed the threshold until the next check. Foreground gateways keep the logging destination chosen by their caller.

## The app

The app is inside this package, at `Apple Bridge.app`. `bridge` finds it on its own. `bridge up` does not start again after a restart; run it again, or use the Apple Bridge app from the project page, which starts at login and updates itself. If that app is already running, `bridge up` reports it and leaves it alone.

Documentation: <https://github.com/JohanSardar/apple-bridge>
