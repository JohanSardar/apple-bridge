use super::{Cli, modes::Modes, ui};
use clap::{Arg, Command, CommandFactory};
use serde_json::{Value, json};

fn push(text: &mut String, piece: &str) {
    text.push_str(piece);
}

pub const DOCS: &str =
    "https://github.com/JohanSardar/apple-bridge/blob/main/docs/reference/cli.md";

pub struct Leaf {
    pub path: &'static str,
    pub examples: &'static [(&'static str, &'static str)],
    pub notes: &'static [&'static str],
    pub schema: &'static [(&'static str, &'static str, bool)],
}

const GROUPS: &[(&str, &[&str])] = &[
    (
        "Getting started",
        &["up", "link", "unlink", "permissions", "stats", "down"],
    ),
    ("Reading data", &["read", "search", "call", "skills"]),
    ("Access", &["agents", "tokens", "connectors", "web"]),
    (
        "Remote and replicas",
        &["tailscale", "relay", "remote", "pair", "join"],
    ),
    (
        "Maintenance",
        &[
            "status",
            "doctor",
            "backup",
            "restore",
            "selftest",
            "shortcuts",
            "completion",
        ],
    ),
];

pub const DEPRECATED: &[(&str, &str)] = &[
    ("remote on", "bridge remote enable"),
    ("remote off", "bridge remote disable"),
    (
        "remote certificate-accept",
        "bridge remote certificates accept",
    ),
    (
        "agents level",
        "bridge agents update <AGENT> --level <LEVEL>",
    ),
    ("connectors pending", "bridge connectors list"),
    ("web lock", "bridge web sessions revoke --all"),
    ("web devices", "bridge web sessions list"),
    ("shortcuts install-pack", "bridge shortcuts install"),
    ("revoke", "bridge tokens revoke"),
    ("scope", "bridge tokens update"),
    ("setup", "bridge selftest messages"),
];

const AGENT_FIELDS: &[(&str, &str, bool)] = &[
    ("id", "string", false),
    ("name", "string", false),
    ("level", "string", false),
    ("scopes", "array", false),
    ("created_at", "string", false),
    ("revoked_at", "string|null", false),
    ("last_used", "object|null", false),
    ("expires_at", "string|null", false),
];

const AGENT_LIST_FIELDS: &[(&str, &str, bool)] = &[
    ("agents", "array", false),
    ("next_cursor", "string|null", false),
    ("id", "string", false),
    ("name", "string", false),
    ("level", "string", false),
    ("scopes", "array", false),
    ("created_at", "string", false),
    ("revoked_at", "string|null", false),
    ("last_used", "object|null", false),
    ("expires_at", "string|null", false),
];

const CREDENTIAL_CHANGE_FIELDS: &[(&str, &str, bool)] = &[
    ("agent", "object", false),
    ("token", "object", false),
    ("replica", "object", false),
    ("previous", "object", false),
    ("outcome", "string", false),
    ("next", "array", false),
    ("id", "string", false),
    ("kind", "string", false),
    ("scopes", "array", false),
    ("remote", "boolean", false),
    ("expiresAt", "string|null", false),
    ("revokedAt", "string|null", false),
];

const REMOTE_FIELDS: &[(&str, &str, bool)] = &[
    ("remote", "object", false),
    ("outcome", "string", false),
    ("next", "array", false),
    ("userActionRequired", "boolean", false),
    ("message", "string", false),
    ("id", "string", false),
    ("state", "string", false),
    ("backend", "string", false),
    ("url", "string", false),
    ("last_error", "string", false),
    ("pending_link", "string", false),
    ("signInUrl", "string", true),
    ("tls_termination", "string", false),
    ("certificate_expiry", "string", false),
];

const LINK_FIELDS: &[(&str, &str, bool)] = &[
    ("outcome", "string", false),
    ("client", "string", false),
    ("name", "string", false),
    ("url", "string", false),
    ("mcp", "string", false),
    ("rest", "string", false),
    ("register", "object", false),
    ("login", "object", false),
    ("next", "array", false),
];

pub const LEAVES: &[Leaf] = &[
    Leaf {
        path: "up",
        examples: &[
            ("Start Apple Bridge on this Mac", "bridge up"),
            (
                "Start it from a script and read the MCP address",
                "bridge up --json",
            ),
        ],
        notes: &[
            "Returns once the gateway is ready; syncing continues in the background. Running it again reuses the running gateway.",
            "Piped, stdout holds only the MCP address; the summary goes to stderr.",
        ],
        schema: &[
            ("state", "string", false),
            ("started", "boolean", false),
            ("mcp", "string", false),
            ("home", "string", false),
            ("helper", "boolean", false),
            ("remote", "string", false),
            ("seconds", "number", false),
            ("restart_needed", "boolean", false),
            ("outcome", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "link",
        examples: &[
            ("Connect Claude Code", "bridge link claude"),
            (
                "Print the commands to run yourself",
                "bridge link codex --print",
            ),
        ],
        notes: &[
            "Opens a browser for sign-in only on a local, interactive terminal; the address is always printed.",
            "Piped, stdout holds only the server name; bridge link other prints only the MCP address.",
        ],
        schema: LINK_FIELDS,
    },
    Leaf {
        path: "tailscale",
        examples: &[
            (
                "Let your devices and Grok Bot reach this Mac",
                "bridge tailscale",
            ),
            ("Turn it off", "bridge tailscale --off"),
        ],
        notes: &[
            "Installs and starts Tailscale if needed, then turns on the Tailscale route. Only devices on your Tailscale network can reach it.",
            "Cloud agents ask at the printed /connect address, and you click Allow on this Mac.",
        ],
        schema: &[
            ("outcome", "string", false),
            ("mcp", "string", false),
            ("connect", "string", false),
        ],
    },
    Leaf {
        path: "relay setup",
        examples: &[(
            "Install a relay on your server and connect this Mac",
            "bridge relay setup root@203.0.113.10 --domain bridge.example.com",
        )],
        notes: &[
            "Shows the two DNS records to add and waits until they are live, then installs the relay over ssh and connects this Mac.",
            "Run it again to pick up where it stopped.",
        ],
        schema: &[],
    },
    Leaf {
        path: "unlink",
        examples: &[("Disconnect Claude Code", "bridge unlink claude")],
        notes: &[
            "This removes the agent's registration. To end its access on this Mac too, revoke it with bridge agents revoke.",
            "Piped, stdout holds only the removed server name.",
        ],
        schema: &[
            ("outcome", "string", false),
            ("client", "string", false),
            ("name", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "stats",
        examples: &[
            ("See what has synced", "bridge stats"),
            (
                "Wait until every source has finished syncing",
                "bridge stats --wait",
            ),
            (
                "Follow sync events for 5 minutes",
                "bridge stats --follow --timeout 5m",
            ),
            (
                "Stream sync events as JSON lines for 5 minutes",
                "bridge stats --follow --timeout 5m --json",
            ),
        ],
        notes: &[
            "Counts only, never content.",
            "--wait exits 0 once every source has finished syncing. It exits 1 when a source needs attention, such as a missing permission, or when --timeout (default 10m) passes first; syncing continues either way.",
            "--follow keeps going until stopped or --timeout; in CI or under an agent it needs --timeout. Reaching --timeout exits 0 because watching was the task.",
            "With --json, --wait and --follow print one JSON event per line; warnings arrive as {\"type\":\"warning\"} lines.",
        ],
        schema: &[("sources", "array", false), ("total", "number", false)],
    },
    Leaf {
        path: "down",
        examples: &[("Stop the gateway before updating", "bridge down")],
        notes: &[
            "Leaves a gateway that something else started, such as the Apple Bridge app, running.",
            "Run bridge down before updating or uninstalling. Moving or deleting the installation does not stop a running gateway.",
        ],
        schema: &[("home", "string", false), ("outcome", "string", false)],
    },
    Leaf {
        path: "permissions guide",
        examples: &[
            ("Walk through each permission", "bridge permissions guide"),
            (
                "Request two permissions from a script",
                "bridge permissions guide --only contacts,calendar",
            ),
        ],
        notes: &[
            "Every step is optional, and permissions you've already allowed are kept. Enter at a prompt continues; s skips; Ctrl+C or end of input stops before the next step.",
            "Without a terminal, pass --only with the permissions to request. Without it, the list with each status is printed and the command exits 2.",
        ],
        schema: &[
            ("permissions", "object", false),
            ("requested", "array", false),
            ("skipped", "array", false),
        ],
    },
    Leaf {
        path: "permissions status",
        examples: &[(
            "Check which permissions are allowed",
            "bridge permissions status",
        )],
        notes: &["With --json, each permission maps to allowed, needs permission or unknown."],
        schema: &[
            ("full_disk_access", "string", false),
            ("contacts", "string", false),
            ("calendar", "string", false),
            ("reminders", "string", false),
            ("photos", "string", false),
            ("screen_recording", "string", false),
            ("camera", "string", false),
            ("microphone", "string", false),
            ("location", "string", false),
        ],
    },
    Leaf {
        path: "permissions request",
        examples: &[(
            "Ask macOS for camera access",
            "bridge permissions request camera",
        )],
        notes: &[
            "Names: contacts, calendar, reminders, photos, location, camera, microphone, screen_recording, speech_recognition, focus, bluetooth, notifications, automation_mail, automation_safari, automation_shortcuts_events.",
        ],
        schema: &[
            ("permission", "string", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "read",
        examples: &[
            ("Show your latest chats", "bridge read chats"),
            (
                "Find messages about dinner",
                "bridge read messages --query dinner --limit 20",
            ),
            ("Read the next page", "bridge read mail --next <CURSOR>"),
        ],
        notes: &[
            "Shows newest first. --limit is the total to show (default 10, cap 200); only what remains is fetched. Photos is queried live and follows its pages like the rest; everything else is read from what has synced, with its freshness on stderr.",
            "With --json, --limit 0 still makes one request at the gateway's page size and prints its envelope without records, as in earlier releases.",
            "--next continues with the same filters, window and order. Options you leave out come from the cursor; ones that differ are refused.",
            "Piped, rows are tab-separated with a header line, even when nothing matches. With --json, --fields picks fields inside each record's data.",
            "Uses a temporary read-only credential that's revoked when the read ends.",
        ],
        schema: &[
            ("next_cursor", "string", false),
            ("chats", "array", false),
            ("items", "array", false),
            ("records", "array", false),
            ("result_json", "object", false),
        ],
    },
    Leaf {
        path: "search",
        examples: &[
            ("Search everything that has synced", "bridge search dentist"),
            (
                "Fetch every match as JSON",
                "bridge search invoice --all --json",
            ),
        ],
        notes: &[
            "--limit is the total to show (default 10, cap 200). --all fetches every page, up to 10,000 results. --next continues the same search; TEXT can be left out.",
            "With --json, --limit 0 still prints one page at the gateway's page size, as in earlier releases.",
            "Piped, rows are tab-separated with a header line, even when nothing matches.",
        ],
        schema: &[
            ("records", "array", false),
            ("next_cursor", "string", false),
        ],
    },
    Leaf {
        path: "call",
        examples: &[
            ("List the operations you can call", "bridge call --list"),
            ("Check every source's state", "bridge call sources.status"),
            (
                "Pass arguments on stdin",
                "echo '{\"limit\": 5}' | bridge call messages.chats --data -",
            ),
        ],
        notes: &[
            "Prints the operation's raw JSON response. Read-only unless --write; other changes go through MCP.",
            "A write whose result can't be confirmed is reported as unknown_outcome with its operation ID; inspect it before trying again. Writes are never retried.",
            "The result is the operation's own output; see its output schema in bridge call --list --json.",
        ],
        schema: &[],
    },
    Leaf {
        path: "skills list",
        examples: &[("List the built-in skills", "bridge skills list")],
        notes: &[],
        schema: &[("name", "string", false), ("summary", "string", false)],
    },
    Leaf {
        path: "skills get",
        examples: &[(
            "Print the core skill for your agent",
            "bridge skills get core",
        )],
        notes: &["Prints the skill's exact Markdown."],
        schema: &[],
    },
    Leaf {
        path: "agents list",
        examples: &[
            ("List agents", "bridge agents list"),
            (
                "List agent IDs as JSON",
                "bridge agents list --json --fields id,name",
            ),
            ("See the next page", "bridge agents list --next <CURSOR>"),
        ],
        notes: &[
            "Lists agents in the order they were created, oldest first, 50 at a time (--limit up to 200). --next continues; the JSON gives next_cursor, null on the last page.",
            "Tokens are never shown again after creation.",
        ],
        schema: AGENT_LIST_FIELDS,
    },
    Leaf {
        path: "agents create",
        examples: &[
            (
                "Create a read-only agent for a bot that stops working in 30 days",
                "bridge agents create --name my-bot --level read --scope messages:read,contacts:read --expires-in 30d",
            ),
            (
                "Create one from a script and keep its ID",
                "bridge agents create --name ci --level read --expires-in 7d --json",
            ),
        ],
        notes: &[
            "The token prints once, alone on stdout. --scope sets the agent's access grants.",
            "Without --expires-in the token never expires. On a terminal you're asked first; without one it's created with a warning.",
        ],
        schema: &[
            ("agent", "object", false),
            ("token", "string", true),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "agents inspect",
        examples: &[("Show one agent", "bridge agents inspect <AGENT>")],
        notes: &[],
        schema: AGENT_FIELDS,
    },
    Leaf {
        path: "agents update",
        examples: &[
            (
                "Make an agent ask before each change",
                "bridge agents update <AGENT> --level approve",
            ),
            (
                "Replace an agent's access",
                "bridge agents update <AGENT> --scope messages:read",
            ),
        ],
        notes: &[
            "A read agent can't be raised to full or approve; revoke it and create a new one.",
            "Raising the level to full or adding sources asks first; without a terminal, pass --yes.",
        ],
        schema: &[
            ("agent", "object", false),
            ("previous", "object", false),
            ("outcome", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "agents revoke",
        examples: &[
            ("Revoke an agent", "bridge agents revoke <AGENT>"),
            (
                "Revoke one from a script",
                "bridge agents revoke <AGENT> --yes",
            ),
        ],
        notes: &[
            "Asks first; without a terminal, pass --yes. The agent stays in the list, marked revoked.",
        ],
        schema: CREDENTIAL_CHANGE_FIELDS,
    },
    Leaf {
        path: "agents level",
        examples: &[],
        notes: &[
            "Like the old bridge scope, it still runs without --yes outside a terminal, with a warning, until the next major version.",
        ],
        schema: AGENT_FIELDS,
    },
    Leaf {
        path: "tokens create",
        examples: &[
            (
                "Create a read-only token for a script that stops working in 12 hours",
                "bridge tokens create --scope reminders:read --expires-in 12h",
            ),
            (
                "Store one in a variable",
                "TOKEN=$(bridge tokens create --scope calendar:read --expires-in 1h)",
            ),
        ],
        notes: &[
            "The token prints once, alone on stdout. With --json it is in secret.",
            "Without --expires-in the token never expires. On a terminal you're asked first; without one it's created with a warning.",
        ],
        schema: &[
            ("token", "object", false),
            ("secret", "string", true),
            ("outcome", "string", false),
            ("next", "array", false),
            ("id", "string", false),
            ("scopes", "array", false),
            ("remote", "boolean", false),
            ("expiresAt", "string|null", false),
        ],
    },
    Leaf {
        path: "tokens revoke",
        examples: &[("Revoke a token", "bridge tokens revoke <CREDENTIAL>")],
        notes: &[
            "Accepts token, agent and replica IDs. Asks first; without a terminal, pass --yes.",
        ],
        schema: CREDENTIAL_CHANGE_FIELDS,
    },
    Leaf {
        path: "tokens update",
        examples: &[(
            "Replace a credential's access",
            "bridge tokens update <CREDENTIAL> --scope messages:read",
        )],
        notes: &[
            "Accepts token, agent and replica IDs. Adding sources asks first; without a terminal, pass --yes.",
        ],
        schema: CREDENTIAL_CHANGE_FIELDS,
    },
    Leaf {
        path: "connectors list",
        examples: &[("See which apps are waiting", "bridge connectors list")],
        notes: &[
            "Lists waiting requests, the one that expires first at the top, 50 at a time (--limit up to 200). --next continues.",
        ],
        schema: &[
            ("connectors", "array", false),
            ("next_cursor", "string|null", false),
            ("id", "string", false),
            ("client_name", "string", false),
            ("redirect_host", "string", false),
            ("access", "string", false),
            ("expires_at", "string", false),
        ],
    },
    Leaf {
        path: "connectors allow",
        examples: &[(
            "Allow an app after checking its code",
            "bridge connectors allow --request <REQUEST> --code <CODE>",
        )],
        notes: &["Only allow an app whose code matches the one on its page."],
        schema: &[("accepted", "boolean", false), ("outcome", "string", false)],
    },
    Leaf {
        path: "connectors deny",
        examples: &[(
            "Deny an app",
            "bridge connectors deny --request <REQUEST> --code <CODE>",
        )],
        notes: &[],
        schema: &[("accepted", "boolean", false), ("outcome", "string", false)],
    },
    Leaf {
        path: "connectors pending",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "web enroll",
        examples: &[
            ("Add a passkey on this Mac", "bridge web enroll"),
            (
                "Add one through remote access",
                "bridge web enroll --public",
            ),
        ],
        notes: &[
            "The link is a one-time secret that works for 10 minutes. It uses the port the gateway listens on. Piped, stdout holds only the link and no QR code is drawn.",
        ],
        schema: &[
            ("url", "string", true),
            ("expires_in_seconds", "number", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "web sessions list",
        examples: &[("List signed-in browsers", "bridge web sessions list")],
        notes: &[
            "Lists browsers in the order they signed in, oldest first, 50 at a time (--limit up to 200). --next continues.",
        ],
        schema: &[
            ("sessions", "array", false),
            ("next_cursor", "string|null", false),
            ("device_id", "string", false),
            ("kind", "string", false),
            ("created_at", "string", false),
            ("last_seen", "string", false),
            ("expires_at", "string", false),
        ],
    },
    Leaf {
        path: "web sessions revoke",
        examples: &[("Sign out every browser", "bridge web sessions revoke --all")],
        notes: &["Asks first; without a terminal, pass --yes."],
        schema: &[("revoked", "number", false), ("outcome", "string", false)],
    },
    Leaf {
        path: "web lock",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "web devices",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "remote enable",
        examples: &[
            (
                "Turn on remote access through the relay",
                "bridge remote enable",
            ),
            (
                "Use your own relay",
                "bridge remote enable --relay-url https://relay.example.com",
            ),
        ],
        notes: &[
            "Waits until remote access is on, up to --timeout (default 10m). Stopping the wait leaves the request with the gateway; check it with bridge remote status.",
            "With --json it doesn't wait for a person: once a sign-in link is needed it prints outcome pending, the link in remote.signInUrl and userActionRequired true.",
            "--no-open never opens a browser; --open opens one even without a terminal, unless over SSH.",
        ],
        schema: REMOTE_FIELDS,
    },
    Leaf {
        path: "remote disable",
        examples: &[("Turn off remote access", "bridge remote disable")],
        notes: &[],
        schema: REMOTE_FIELDS,
    },
    Leaf {
        path: "remote status",
        examples: &[
            ("Check remote access", "bridge remote status"),
            (
                "Print agent connection snippets",
                "bridge remote status --snippets",
            ),
        ],
        notes: &[
            "With --json the state is under remote. Piped without --json it prints the state alone, as before. --snippets --json gives {\"snippets\": TEXT}.",
        ],
        schema: &[
            ("remote", "object", false),
            ("snippets", "string", false),
            ("id", "string", false),
            ("state", "string", false),
            ("backend", "string", false),
            ("url", "string", false),
            ("last_error", "string", false),
            ("pending_link", "string", false),
            ("tls_termination", "string", false),
            ("certificate_expiry", "string", false),
        ],
    },
    Leaf {
        path: "remote key",
        examples: &[(
            "Print the key for your relay's allowlist",
            "bridge remote key",
        )],
        notes: &["Prints only the 43-character key."],
        schema: &[("key", "string", false)],
    },
    Leaf {
        path: "remote certificates accept",
        examples: &[(
            "Accept a certificate you requested",
            "bridge remote certificates accept <FINGERPRINT>",
        )],
        notes: &[],
        schema: &[
            ("fingerprint", "string", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "remote on",
        examples: &[],
        notes: &[
            "Prints the sign-in link and the address on stdout and opens the link in a browser unless --no-open, with or without a terminal.",
        ],
        schema: REMOTE_FIELDS,
    },
    Leaf {
        path: "remote off",
        examples: &[],
        notes: &[],
        schema: REMOTE_FIELDS,
    },
    Leaf {
        path: "remote certificate-accept",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "pair",
        examples: &[
            (
                "Create a code for a replica",
                "bridge pair --scope messages:read",
            ),
            (
                "Pair an iPhone through remote access",
                "bridge pair --iphone --via-remote",
            ),
        ],
        notes: &[
            "The code is a secret that works until --expires-in passes. Piped, stdout holds only the code.",
        ],
        schema: &[
            ("code", "string", true),
            ("expires_in_seconds", "number", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "join",
        examples: &[("Join a gateway as a replica", "bridge join <CODE>")],
        notes: &["Piped, stdout holds only the replica's home."],
        schema: &[("home", "string", false), ("outcome", "string", false)],
    },
    Leaf {
        path: "status",
        examples: &[("Show this home's role", "bridge status")],
        notes: &[],
        schema: &[("role", "string", false), ("cursor", "object", false)],
    },
    Leaf {
        path: "doctor",
        examples: &[
            ("Check this home", "bridge doctor"),
            ("Check it from a script", "bridge doctor --json"),
        ],
        notes: &[
            "A finished report exits 0, even when a check fails. Pass --check to exit 1 with unhealthy_home instead.",
        ],
        schema: &[
            ("ok", "boolean", false),
            ("applied", "number", false),
            ("checks", "object", false),
            ("clock_skew", "string", false),
            ("credentials", "object", false),
            ("cursor", "object", false),
            ("disk_encryption", "string", false),
            ("document_checks", "object", false),
            ("exactly_once", "boolean", false),
            ("floor", "number", false),
            ("private_modes", "boolean", false),
            ("records", "object", false),
            ("schema", "string", false),
            ("shape", "object", false),
            ("sqlite", "string", false),
            ("versions", "object", false),
            ("warnings", "array", false),
        ],
    },
    Leaf {
        path: "backup",
        examples: &[
            ("Back up this home", "bridge backup ~/apple-bridge.db"),
            (
                "Replace an older backup from a script",
                "bridge backup ~/apple-bridge.db --force",
            ),
        ],
        notes: &[
            "Refuses an existing file unless --force, or you confirm at the prompt. The backup is written privately and moved into place whole.",
        ],
        schema: &[
            ("path", "string", false),
            ("bytes", "number", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "restore",
        examples: &[
            ("Restore a backup", "bridge restore ~/apple-bridge.db"),
            (
                "Restore from a script after reviewing the effects",
                "bridge restore ~/apple-bridge.db --yes",
            ),
        ],
        notes: &[
            "Stop the gateway first with bridge down. Shows what will be replaced and asks; without a terminal, pass --yes.",
        ],
        schema: &[
            ("home", "string", false),
            ("backup", "string", false),
            ("outcome", "string", false),
        ],
    },
    Leaf {
        path: "selftest messages",
        examples: &[(
            "Prove Messages works on your own chat",
            "bridge selftest messages",
        )],
        notes: &["This sends real messages to your own chat through the Helper."],
        schema: &[],
    },
    Leaf {
        path: "shortcuts install",
        examples: &[("Open the Shortcuts pack", "bridge shortcuts install")],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "shortcuts install-pack",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "completion",
        examples: &[(
            "Add completion to zsh",
            "bridge completion zsh > ~/.zfunc/_bridge",
        )],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "help",
        examples: &[
            ("Search commands offline", "bridge help search token"),
            ("List every command as JSON", "bridge help --all --json"),
        ],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "start",
        examples: &[(
            "Run the gateway in the foreground",
            "bridge start --role gateway --api-addr 127.0.0.1:7780 --peer-addr 127.0.0.1:7443",
        )],
        notes: &["Used by the Apple Bridge app's background service."],
        schema: &[],
    },
    Leaf {
        path: "connect",
        examples: &[],
        notes: &[],
        schema: &[],
    },
    Leaf {
        path: "install-credentials",
        examples: &[],
        notes: &["Used by the Apple Bridge app."],
        schema: &[
            ("outcome", "string", false),
            ("home", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "revoke",
        examples: &[],
        notes: &[],
        schema: &[
            ("id", "string", false),
            ("kind", "string", false),
            ("outcome", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "scope",
        examples: &[],
        notes: &[],
        schema: &[
            ("id", "string", false),
            ("scopes", "array", false),
            ("previous", "array", false),
            ("outcome", "string", false),
            ("next", "array", false),
        ],
    },
    Leaf {
        path: "setup",
        examples: &[],
        notes: &["This sends real messages to your own chat through the Helper."],
        schema: &[],
    },
];

pub fn leaf(path: &str) -> Option<&'static Leaf> {
    LEAVES.iter().find(|leaf| leaf.path == path)
}

pub fn deprecated(path: &str) -> Option<&'static str> {
    DEPRECATED
        .iter()
        .find(|(old, _)| *old == path)
        .map(|(_, replacement)| *replacement)
}

pub fn find(path: &[String]) -> Option<Command> {
    let mut command = Cli::command();
    command.build();
    for segment in path {
        let child = command
            .get_subcommands()
            .find(|child| {
                child.get_name() == segment || child.get_all_aliases().any(|alias| alias == segment)
            })?
            .clone();
        command = child;
    }
    Some(command)
}

fn summary(command: &Command) -> String {
    let text = command
        .get_about()
        .map(ToString::to_string)
        .unwrap_or_default();
    text.split(" (deprecated")
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn local_options(command: &Command, root: bool) -> Vec<&Arg> {
    let mut options: Vec<&Arg> = command
        .get_arguments()
        .filter(|argument| !argument.is_positional() && !argument.is_hide_set())
        .filter(|argument| root || !argument.is_global_set())
        .collect();
    options.sort_by_key(|argument| argument.get_long().unwrap_or_default().to_owned());
    options
}

fn positionals(command: &Command) -> Vec<&Arg> {
    command
        .get_arguments()
        .filter(|argument| argument.is_positional() && !argument.is_hide_set())
        .collect()
}

fn value_hint(argument: &Arg) -> Option<String> {
    if !argument.get_action().takes_values() {
        return None;
    }
    if argument.get_long() == Some("non-interactive") {
        return None;
    }
    argument
        .get_value_names()
        .and_then(|names| names.first())
        .map(|name| format!("<{name}>"))
}

fn syntax(argument: &Arg) -> String {
    let mut text = String::new();
    if let Some(short) = argument.get_short() {
        push(&mut text, &format!("-{short}, "));
    }
    if let Some(long) = argument.get_long() {
        push(&mut text, &format!("--{long}"));
    }
    if let Some(hint) = value_hint(argument) {
        text.push(' ');
        text.push_str(&hint);
    }
    text
}

fn possible(argument: &Arg) -> Vec<String> {
    argument
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_owned())
        .filter(|value| value != "true" && value != "false")
        .collect()
}

fn description(argument: &Arg) -> String {
    let mut text = argument
        .get_help()
        .map(ToString::to_string)
        .unwrap_or_default();
    let values = possible(argument);
    if !values.is_empty() && !values.iter().all(|value| text.contains(value.as_str())) {
        push(&mut text, &format!(" (accepted: {})", values.join(", ")));
    }
    let defaults: Vec<String> = argument
        .get_default_values()
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
        .collect();
    if !defaults.is_empty() && !text.contains("(default") && argument.get_action().takes_values() {
        push(&mut text, &format!(" (default: {})", defaults.join(",")));
    }
    text
}

fn usage(path: &[String], command: &Command) -> String {
    let mut line = format!("bridge {}", path.join(" ")).trim_end().to_owned();
    if command.has_subcommands() {
        line.push_str(" <command>");
    }
    for argument in positionals(command) {
        let name = argument
            .get_value_names()
            .and_then(|names| names.first())
            .map_or_else(|| argument.get_id().to_string(), ToString::to_string)
            .to_uppercase();
        let multiple = argument
            .get_num_args()
            .is_some_and(|range| range.max_values() > 1);
        let ellipsis = if multiple { "..." } else { "" };
        if argument.is_required_set() {
            push(&mut line, &format!(" <{name}>{ellipsis}"));
        } else {
            push(&mut line, &format!(" [{name}]{ellipsis}"));
        }
    }
    line.push_str(" [options]");
    line
}

fn rows(lines: &mut Vec<String>, entries: &[(String, String)], width: usize) {
    let column = entries
        .iter()
        .map(|(syntax, _)| ui::display_width(syntax))
        .max()
        .unwrap_or(0)
        + 2;
    for (syntax, text) in entries {
        let start = lines.len();
        if column + 2 + 20 > width {
            lines.push(format!("  {syntax}"));
            lines.extend(ui::wrap(text, width, "      ", "      "));
        } else {
            let first = format!("  {syntax:<column$}");
            let rest = " ".repeat(column + 2);
            lines.extend(
                ui::wrap(text, width, &first, &rest)
                    .into_iter()
                    .map(|line| {
                        if line.trim().is_empty() {
                            line.trim_end().to_owned()
                        } else {
                            line
                        }
                    }),
            );
        }
        if let Some(line) = lines.get_mut(start)
            && let Some(after) = line.strip_prefix(&format!("  {syntax}"))
        {
            *line = format!("  {}{after}", ui::bold(syntax));
        }
    }
}

fn path_key(path: &[String]) -> String {
    path.join(" ")
}

pub fn anchor(path: &[String]) -> String {
    if path.is_empty() {
        "bridge".to_owned()
    } else {
        format!("bridge-{}", path.join("-"))
    }
}

pub fn learn_more(path: &[String]) -> String {
    format!("{DOCS}#{}", anchor(path))
}

pub fn render(path: &[String]) -> String {
    render_with(path, Modes::get().width())
}

#[allow(clippy::too_many_lines)]
pub fn render_with(path: &[String], width: usize) -> String {
    let Some(command) = find(path) else {
        return String::new();
    };
    let root = path.is_empty();
    let key = path_key(path);
    let mut lines = vec![usage(path, &command)];
    lines.extend(ui::wrap(&summary(&command), width, "", ""));
    if let Some(replacement) = deprecated(&key) {
        lines.push(format!("Deprecated: use {replacement}."));
    }
    let children: Vec<&Command> = command
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
        .collect();
    if root {
        for (title, names) in GROUPS {
            lines.push(String::new());
            lines.push(ui::heading(&format!("{title}:")));
            let entries: Vec<(String, String)> = names
                .iter()
                .filter_map(|name| children.iter().find(|child| child.get_name() == *name))
                .map(|child| (child.get_name().to_owned(), summary(child)))
                .collect();
            rows(&mut lines, &entries, width);
        }
    } else if !children.is_empty() {
        lines.push(String::new());
        lines.push(ui::heading("Commands:"));
        let entries: Vec<(String, String)> = children
            .iter()
            .map(|child| (child.get_name().to_owned(), summary(child)))
            .collect();
        rows(&mut lines, &entries, width);
    }
    let arguments: Vec<(String, String)> = positionals(&command)
        .into_iter()
        .map(|argument| {
            let name = argument
                .get_value_names()
                .and_then(|names| names.first())
                .map_or_else(|| argument.get_id().to_string(), ToString::to_string)
                .to_uppercase();
            (format!("<{name}>"), description(argument))
        })
        .collect();
    if !arguments.is_empty() {
        lines.push(String::new());
        lines.push(ui::heading("Arguments:"));
        rows(&mut lines, &arguments, width);
    }
    let options: Vec<(String, String)> = local_options(&command, false)
        .into_iter()
        .map(|argument| (syntax(argument), description(argument)))
        .collect();
    if !options.is_empty() && !root {
        lines.push(String::new());
        lines.push(ui::heading("Options:"));
        rows(&mut lines, &options, width);
    }
    lines.push(String::new());
    if root {
        lines.push(ui::heading("Global options:"));
        let globals: Vec<(String, String)> = local_options(&command, true)
            .into_iter()
            .map(|argument| (syntax(argument), description(argument)))
            .collect();
        rows(&mut lines, &globals, width);
    } else {
        lines.push("Global options: see bridge --help".to_owned());
    }
    let leaf = leaf(&key);
    let examples: &[(&str, &str)] = if root {
        &[
            ("Start Apple Bridge on this Mac", "bridge up"),
            ("Then connect your agent", "bridge link claude"),
        ]
    } else {
        leaf.map_or(&[], |leaf| leaf.examples)
    };
    if !examples.is_empty() {
        lines.push(String::new());
        lines.push(ui::heading("Examples:"));
        for (index, (text, example)) in examples.iter().enumerate() {
            if index > 0 {
                lines.push(String::new());
            }
            lines.push(format!("  {text}"));
            lines.push(format!("    {example}"));
        }
    }
    if let Some(leaf) = leaf
        && !leaf.notes.is_empty()
    {
        lines.push(String::new());
        lines.push(ui::heading("Notes:"));
        for note in leaf.notes {
            lines.extend(ui::wrap(note, width, "  ", "  "));
        }
    }
    lines.push(String::new());
    lines.push(format!("Learn more: {}", learn_more(path)));
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn option_json(argument: &Arg) -> Value {
    let kind = if argument.get_action().takes_values() {
        if argument.get_long() == Some("non-interactive") {
            "boolean"
        } else {
            "string"
        }
    } else {
        "boolean"
    };
    json!({
        "name": argument.get_long().map(|long| format!("--{long}")),
        "short": argument.get_short().map(|short| format!("-{short}")),
        "aliases": argument.get_all_aliases().unwrap_or_default().iter().map(|alias| format!("--{alias}")).collect::<Vec<_>>(),
        "type": kind,
        "value_hint": value_hint(argument),
        "default": argument.get_default_values().first().map(|value| value.to_string_lossy().into_owned()).filter(|value| !value.is_empty()),
        "values": possible(argument),
        "repeatable": false,
        "required": argument.is_required_set(),
        "global": argument.is_global_set(),
        "summary": argument.get_help().map(ToString::to_string).unwrap_or_default(),
    })
}

pub fn document(path: &[String]) -> Value {
    let Some(command) = find(path) else {
        return Value::Null;
    };
    let key = path_key(path);
    let leaf = leaf(&key);
    json!({
        "path": format!("bridge {key}").trim_end(),
        "summary": summary(&command),
        "usage": usage(path, &command),
        "aliases": command.get_all_aliases().collect::<Vec<_>>(),
        "deprecated": deprecated(&key),
        "hidden": command.is_hide_set(),
        "lifecycle": if deprecated(&key).is_some() { "deprecated" } else if command.is_hide_set() { "hidden" } else { "stable" },
        "subcommands": command.get_subcommands().filter(|child| !child.is_hide_set()).map(|child| json!({"name": child.get_name(), "summary": summary(child)})).collect::<Vec<_>>(),
        "arguments": positionals(&command).into_iter().map(|argument| json!({
            "name": argument.get_value_names().and_then(|names| names.first()).map(ToString::to_string),
            "required": argument.is_required_set(),
            "values": possible(argument),
            "summary": argument.get_help().map(ToString::to_string).unwrap_or_default(),
        })).collect::<Vec<_>>(),
        "options": local_options(&command, path.is_empty()).into_iter().map(option_json).collect::<Vec<_>>(),
        "examples": leaf.map(|leaf| leaf.examples.iter().map(|(summary, command)| json!({"summary": summary, "command": command})).collect::<Vec<_>>()).unwrap_or_default(),
        "notes": leaf.map(|leaf| leaf.notes.to_vec()).unwrap_or_default(),
        "result_schema": result_schema(&key, leaf),
        "learn_more": learn_more(path),
    })
}

fn result_schema(key: &str, leaf: Option<&Leaf>) -> Value {
    if key == "call" {
        return json!({"from": "operation", "see": "bridge call --list --json", "field": "output"});
    }
    let Some(leaf) = leaf.filter(|leaf| !leaf.schema.is_empty()) else {
        return Value::Null;
    };
    let mut schema = json!({
        "fields": leaf.schema.iter().map(|(name, kind, secret)| json!({"name": name, "type": kind, "secret": secret})).collect::<Vec<_>>()
    });
    if key == "read" {
        schema["varies_by"] = json!("KIND");
        schema["note"] = json!(
            "Each kind returns the fields its operation returns, plus next_cursor, which is null on the last page."
        );
    }
    schema
}

pub fn every_path() -> Vec<Vec<String>> {
    fn walk(command: &Command, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(path.clone());
        for child in command.get_subcommands() {
            path.push(child.get_name().to_owned());
            walk(child, path, out);
            path.pop();
        }
    }
    let mut command = Cli::command();
    command.build();
    let mut out = Vec::new();
    walk(&command, &mut Vec::new(), &mut out);
    out
}

pub fn all_document() -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commands": every_path().iter().map(|path| document(path)).collect::<Vec<_>>(),
    })
}

pub fn search(query: &str) -> Vec<(String, String)> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for path in every_path() {
        let Some(command) = find(&path) else {
            continue;
        };
        if path.is_empty()
            || command.is_hide_set()
            || path.iter().any(|segment| {
                find(&path[..=path.iter().position(|s| s == segment).unwrap_or(0)])
                    .is_some_and(|c| c.is_hide_set())
            })
        {
            continue;
        }
        let key = path_key(&path);
        let mut haystack = format!("{key} {}", summary(&command)).to_lowercase();
        if let Some(leaf) = leaf(&key) {
            for (text, example) in leaf.examples {
                haystack.push(' ');
                haystack.push_str(&text.to_lowercase());
                haystack.push(' ');
                haystack.push_str(&example.to_lowercase());
            }
        }
        if terms.iter().all(|term| haystack.contains(term.as_str())) {
            matches.push((format!("bridge {key}"), summary(&command)));
        }
        if matches.len() == 10 {
            break;
        }
    }
    matches
}

pub fn suggestion(candidates: &[String], typed: &str) -> Option<String> {
    let mut best: Option<((usize, usize), &String)> = None;
    for candidate in candidates {
        let distance = edit_distance(typed, candidate);
        let limit = (candidate.len().max(typed.len()) / 3).max(1);
        let rank = (distance, candidate.len().abs_diff(typed.len()));
        if distance <= limit && best.is_none_or(|(current, _)| rank < current) {
            best = Some((rank, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.clone())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in table[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[i - 1][j] + 1)
                .min(table[i][j - 1] + 1)
                .min(table[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[i - 2][j - 2] + 1);
            }
            table[i][j] = best;
        }
    }
    table[a.len()][b.len()]
}

pub fn child_names(path: &[String]) -> Vec<String> {
    find(path)
        .map(|command| {
            command
                .get_subcommands()
                .filter(|child| !child.is_hide_set())
                .map(|child| child.get_name().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub fn option_names(path: &[String]) -> Vec<String> {
    let mut names: Vec<String> = find(path)
        .map(|command| {
            command
                .get_arguments()
                .filter(|argument| !argument.is_hide_set())
                .filter_map(|argument| argument.get_long().map(|long| format!("--{long}")))
                .collect()
        })
        .unwrap_or_default();
    let root = Cli::command();
    names.extend(
        root.get_arguments()
            .filter_map(|argument| argument.get_long().map(|long| format!("--{long}"))),
    );
    names.sort();
    names.dedup();
    names
}

pub fn completion(shell: super::tree::Shell) -> String {
    let mut table = Vec::new();
    let mut valued: Vec<(String, String, String)> = Vec::new();
    for path in every_path() {
        if find(&path).is_some_and(|command| command.is_hide_set()) {
            continue;
        }
        let mut words = child_names(&path);
        words.extend(option_names(&path));
        if let Some(command) = find(&path) {
            for argument in positionals(&command) {
                words.extend(possible(argument));
            }
        }
        table.push((path.join(" "), words.join(" ")));
        if let Some(command) = find(&path) {
            for argument in local_options(&command, path.is_empty()) {
                let choices = possible(argument);
                if let Some(long) = argument.get_long()
                    && !choices.is_empty()
                {
                    valued.push((path.join(" "), long.to_owned(), choices.join(" ")));
                }
            }
        }
    }
    match shell {
        super::tree::Shell::Bash | super::tree::Shell::Zsh => {
            let mut cases = String::new();
            for (path, words) in &table {
                push(
                    &mut cases,
                    &format!("    \"{path}\") words=\"{words}\" ;;\n"),
                );
            }
            let mut value_cases = String::new();
            for (path, long, values) in &valued {
                push(
                    &mut value_cases,
                    &format!("    \"{path}|--{long}\") words=\"{values}\" ;;\n"),
                );
            }
            let prelude = if shell == super::tree::Shell::Zsh {
                "#compdef bridge\nautoload -U +X bashcompinit && bashcompinit\n"
            } else {
                ""
            };
            format!(
                "{prelude}_bridge() {{\n  local current=\"${{COMP_WORDS[COMP_CWORD]}}\" path=\"\" words=\"\" index candidate\n  for ((index = 1; index < COMP_CWORD; index++)); do\n    candidate=\"${{COMP_WORDS[index]}}\"\n    case \"$candidate\" in -*) continue ;; esac\n    case \"$(_bridge_words \"$path\")\" in *\" $candidate \"*) path=\"${{path:+$path }}$candidate\" ;; esac\n  done\n  words=\"$(_bridge_values \"$path|${{COMP_WORDS[COMP_CWORD-1]}}\")\"\n  case \"$words\" in *[!\\ ]*) ;; *) words=\"$(_bridge_words \"$path\")\" ;; esac\n  COMPREPLY=($(compgen -W \"$words\" -- \"$current\"))\n}}\n_bridge_values() {{\n  local words=\"\"\n  case \"$1\" in\n{value_cases}  esac\n  printf ' %s ' \"$words\"\n}}\n_bridge_words() {{\n  local words=\"\"\n  case \"$1\" in\n{cases}  esac\n  printf ' %s ' \"$words\"\n}}\ncomplete -F _bridge bridge\n"
            )
        }
        super::tree::Shell::Fish => {
            let mut script = String::from("complete -c bridge -f\n");
            for (path, words) in &table {
                let condition = if path.is_empty() {
                    "__fish_use_subcommand".to_owned()
                } else {
                    format!(
                        "__fish_seen_subcommand_from {}",
                        path.split(' ').next_back().unwrap_or_default()
                    )
                };
                for word in words.split(' ').filter(|word| !word.is_empty()) {
                    if let Some(long) = word.strip_prefix("--") {
                        push(
                            &mut script,
                            &format!("complete -c bridge -n '{condition}' -l {long}\n"),
                        );
                    } else {
                        push(
                            &mut script,
                            &format!("complete -c bridge -n '{condition}' -a {word}\n"),
                        );
                    }
                }
            }
            for (path, long, values) in &valued {
                let condition = if path.is_empty() {
                    "true".to_owned()
                } else {
                    format!(
                        "__fish_seen_subcommand_from {}",
                        path.split(' ').next_back().unwrap_or_default()
                    )
                };
                push(
                    &mut script,
                    &format!("complete -c bridge -n '{condition}' -l {long} -xa '{values}'\n"),
                );
            }
            script
        }
    }
}

#[cfg(test)]
pub fn markdown() -> String {
    let mut text = String::from(
        "# bridge command reference\n\nGenerated from the CLI's own metadata by `bridge help`. Every command below also accepts the global options listed under `bridge`.\n",
    );
    for path in every_path() {
        let Some(command) = find(&path) else {
            continue;
        };
        if command.is_hide_set()
            && deprecated(&path_key(&path)).is_none()
            && !matches!(
                path_key(&path).as_str(),
                "start" | "connect" | "install-credentials"
            )
        {
            continue;
        }
        let title = format!("bridge {}", path.join(" ")).trim_end().to_owned();
        push(&mut text, &format!("\n## {title}\n\n```text\n"));
        let rendered = render_with(&path, 100);
        let without_link: Vec<&str> = rendered
            .lines()
            .filter(|line| !line.starts_with("Learn more:"))
            .collect();
        text.push_str(without_link.join("\n").trim_end());
        text.push_str("\n```\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_leaf_exists_and_every_example_parses() {
        for leaf in LEAVES {
            let path: Vec<String> = leaf.path.split(' ').map(str::to_owned).collect();
            assert!(find(&path).is_some(), "{}", leaf.path);
            for (_, example) in leaf.examples {
                let words = shell_words(example);
                let start = words.iter().position(|word| word == "bridge").unwrap_or(0);
                let argv: Vec<String> = words[start..]
                    .iter()
                    .take_while(|word| *word != ">")
                    .map(|word| {
                        word.replace("<AGENT>", "agent:01J0000000000000000000000")
                            .replace("<CREDENTIAL>", "abt_1")
                            .replace("<CURSOR>", "c")
                            .replace("<REQUEST>", "r")
                            .replace("<CODE>", "1234")
                            .replace("<FINGERPRINT>", &"ab".repeat(32))
                            .replace("<LEVEL>", "full")
                    })
                    .collect();
                let parsed = <Cli as clap::Parser>::try_parse_from(&argv);
                assert!(parsed.is_ok(), "{example}: {parsed:?}");
            }
        }
    }

    fn shell_words(text: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut quote = None;
        for character in text.chars() {
            match (quote, character) {
                (None, '\'' | '"') => quote = Some(character),
                (Some(open), c) if c == open => quote = None,
                (None, ' ' | '|' | '(' | ')' | '$') => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                (_, c) => current.push(c),
            }
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }

    #[test]
    fn every_visible_leaf_has_examples_and_a_learn_more_link() {
        for path in every_path() {
            let Some(command) = find(&path) else { continue };
            if path.is_empty() || command.has_subcommands() || command.is_hide_set() {
                continue;
            }
            let hidden_parent =
                (1..path.len()).any(|end| find(&path[..end]).is_some_and(|c| c.is_hide_set()));
            if hidden_parent {
                continue;
            }
            let key = path.join(" ");
            let leaf = leaf(&key).unwrap_or_else(|| panic!("no help metadata for {key}"));
            assert!(!leaf.examples.is_empty(), "{key} has no examples");
            assert!(render(&path).contains("Learn more: "), "{key}");
        }
    }

    #[test]
    fn suggestions_offer_at_most_one_close_name() {
        let names: Vec<String> = ["status", "stats", "start", "setup"]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(suggestion(&names, "stauts"), Some("status".to_owned()));
        assert_eq!(suggestion(&names, "zzzzzz"), None);
    }

    #[test]
    fn root_help_groups_commands_by_task_without_a_help_row() {
        let text = render(&[]);
        for title in [
            "Getting started:",
            "Reading data:",
            "Access:",
            "Remote and replicas:",
            "Maintenance:",
        ] {
            assert!(text.contains(title), "{title}");
        }
        assert!(!text.contains("\n  help "));
        assert!(!text.contains("install-credentials"));
        assert!(!text.contains("For more information"));
    }

    #[test]
    fn the_reference_document_matches_the_metadata() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/reference/cli.md");
        let generated = markdown();
        if std::env::var_os("UPDATE_CLI_SNAPSHOT").is_some() {
            let _ = std::fs::write(&path, &generated);
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == generated,
            "docs/reference/cli.md is stale; run UPDATE_CLI_SNAPSHOT=1 cargo test -p ab-runtime"
        );
    }

    #[test]
    #[cfg(not(feature = "lab-harness"))]
    fn the_surface_snapshot_matches_the_metadata() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/reference/cli-surface.json");
        let mut generated = serde_json::to_string_pretty(&all_document()).unwrap_or_default();
        generated.push('\n');
        if std::env::var_os("UPDATE_CLI_SNAPSHOT").is_some() {
            let _ = std::fs::write(&path, &generated);
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == generated,
            "docs/reference/cli-surface.json is stale; run UPDATE_CLI_SNAPSHOT=1 cargo test -p ab-runtime"
        );
    }
}
