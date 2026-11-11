use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub const DEFAULT_GRANTS: &str = "reminders:read,system:read";
pub const DEFAULT_AGENT_GRANTS: &str = "reminders:read";

#[derive(Debug, Parser)]
#[command(
    name = "bridge",
    about = "Connect local agents such as Claude Code and Codex to your Mac's Apple data",
    disable_help_flag = true,
    disable_help_subcommand = true,
    disable_version_flag = true
)]
pub struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Args, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct Globals {
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        env = "APPLE_BRIDGE_HOME",
        hide_env = true,
        help = "Data folder (default: ~/Library/Application Support/AppleBridge; env: APPLE_BRIDGE_HOME)"
    )]
    pub home: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        help = "Run as if started in this folder; relative paths resolve against it"
    )]
    pub cwd: Option<PathBuf>,
    #[arg(long, global = true, help = "Print the result as JSON; never prompt")]
    pub json: bool,
    #[arg(
        long,
        global = true,
        value_name = "FIELDS",
        help = "With --json: keep only these comma-separated fields"
    )]
    pub fields: Option<String>,
    #[arg(
        long,
        global = true,
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "true",
        value_name = "BOOL",
        help = "Never prompt; fail with the input that is needed instead"
    )]
    pub non_interactive: Option<bool>,
    #[arg(
        short = 'y',
        long,
        global = true,
        help = "Confirm destructive changes without asking"
    )]
    pub yes: bool,
    #[arg(long, global = true, help = "Disable color (also NO_COLOR)")]
    pub no_color: bool,
    #[arg(
        long,
        global = true,
        help = "Plain text for screen readers: no color or symbols, named steps, numbered choices (also APPLE_BRIDGE_ACCESSIBLE=1)"
    )]
    pub accessible: bool,
    #[arg(
        short = 'd',
        long,
        global = true,
        help = "Print redacted diagnostic detail on stderr"
    )]
    pub debug: bool,
    #[arg(
        short = 'v',
        long,
        global = true,
        visible_short_alias = 'V',
        action = ArgAction::SetTrue,
        help = "Print the version"
    )]
    pub version: bool,
    #[arg(short = 'h', long, global = true, action = ArgAction::SetTrue, help = "Show help for the command")]
    pub help: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Start Apple Bridge on this Mac for local agents")]
    Up(UpArgs),
    #[command(about = "Connect Claude Code or Codex to this Mac")]
    Link(LinkArgs),
    #[command(about = "Disconnect Claude Code or Codex")]
    Unlink(UnlinkArgs),
    #[command(about = "Check or allow the Mac permissions Apple Bridge reads with")]
    Permissions(PermissionsCommand),
    #[command(about = "Show how much each source has synced, as counts only")]
    Stats(StatsArgs),
    #[command(about = "Stop a gateway that bridge up started")]
    Down,
    #[command(about = "Read synced data in the terminal")]
    Read(ReadArgs),
    #[command(about = "Search synced data")]
    Search(SearchArgs),
    #[command(about = "Call a read operation and print its raw JSON")]
    Call(CallArgs),
    #[command(about = "Print the agent instructions built into this binary")]
    Skills(SkillsCommand),
    #[command(about = "Create, inspect, update or revoke agents", alias = "agent")]
    Agents(AgentsCommand),
    #[command(about = "Create, update or revoke scoped API tokens", alias = "token")]
    Tokens(TokensCommand),
    #[command(about = "Allow or deny apps waiting to connect", alias = "connector")]
    Connectors(ConnectorsCommand),
    #[command(about = "Manage browser sign-in for this Mac's local pages")]
    Web(WebCommand),
    #[command(about = "Let your other devices and cloud agents reach this Mac through Tailscale")]
    Tailscale(TailscaleArgs),
    #[command(about = "Use Apple Bridge from other computers; off until you enable it")]
    Remote(RemoteCommand),
    #[command(about = "Create a pairing code for a replica or iPhone")]
    Pair(PairArgs),
    #[command(about = "Join a gateway as a replica with a pairing code")]
    Join(JoinArgs),
    #[command(about = "Show this home's role and sync position")]
    Status,
    #[command(about = "Check this home's data, files and credentials")]
    Doctor(DoctorArgs),
    #[command(about = "Copy this home's data to a file")]
    Backup(BackupArgs),
    #[command(about = "Replace this home's data with a backup")]
    Restore(RestoreArgs),
    #[command(about = "Run a self-test that sends real messages to yourself")]
    Selftest(SelftestCommand),
    #[command(about = "Install the Shortcuts pack")]
    Shortcuts(ShortcutsCommand),
    #[command(about = "Print a shell completion script")]
    Completion(CompletionArgs),
    #[command(about = "Show help, list every command or search them offline")]
    Help(HelpArgs),
    #[command(
        hide = true,
        about = "Run the gateway in the foreground; bridge up is usually easier"
    )]
    Start(StartArgs),
    #[command(hide = true, about = "Run the private replica client")]
    Connect(ConnectArgs),
    #[command(hide = true, about = "Prepare credentials for the background service")]
    InstallCredentials,
    #[command(
        hide = true,
        about = "Revoke an agent or token (deprecated: bridge tokens revoke)"
    )]
    Revoke(CredentialArgs),
    #[command(
        hide = true,
        about = "Change a token's access (deprecated: bridge tokens update)"
    )]
    Scope(ScopeArgs),
    #[command(
        hide = true,
        about = "Run the Messages self-test, which sends real messages to yourself (deprecated: bridge selftest messages)"
    )]
    Setup(SelftestArgs),
    #[cfg(feature = "lab-harness")]
    #[command(name = "lab-operation-catalog", hide = true)]
    LabOperationCatalog,
    #[cfg(feature = "lab-harness")]
    #[command(name = "lab-credential-lifecycle", hide = true)]
    LabCredentialLifecycle,
    #[cfg(feature = "lab-harness")]
    #[command(name = "lab-session", hide = true)]
    LabSession,
    #[cfg(feature = "lab-harness")]
    #[command(name = "lab-principal", hide = true)]
    LabPrincipal(LabPrincipalArgs),
}

#[derive(Debug, Args)]
pub struct UpArgs {
    #[arg(
        long,
        value_name = "ADDRESS",
        help = "Local API address, loopback only (default: 127.0.0.1:7780)"
    )]
    pub api_addr: Option<String>,
    #[arg(
        long,
        value_name = "ADDRESS",
        help = "Replica feed address (default: 127.0.0.1:7443, or a free port if it's taken)"
    )]
    pub peer_addr: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Helper app (default: the app this binary ships in)"
    )]
    pub helper_app: Option<String>,
    #[arg(long, hide = true)]
    pub no_watch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LinkClient {
    #[value(alias = "claude-code", help = "Claude Code")]
    Claude,
    #[value(help = "Codex")]
    Codex,
    #[value(
        name = "claude-app",
        alias = "claude-desktop",
        help = "The Claude app or claude.ai, through a custom connector"
    )]
    ClaudeApp,
    #[value(help = "Print what any other app or bot needs")]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LinkScope {
    User,
    Project,
    Local,
}

impl LinkScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Args)]
pub struct LinkArgs {
    #[arg(value_enum, value_name = "CLIENT", help = "Which agent to connect")]
    pub client: LinkClient,
    #[arg(
        long,
        value_enum,
        value_name = "SCOPE",
        default_value = "user",
        help = "Claude Code only: where Claude Code saves the registration. This isn't access; you grant access when you sign in"
    )]
    pub scope: LinkScope,
    #[arg(
        long,
        value_name = "NAME",
        default_value = "apple-bridge",
        help = "Name shown in the agent"
    )]
    pub name: String,
    #[arg(long, help = "Print the agent's commands instead of running them")]
    pub print: bool,
    #[arg(long, help = "Register only; sign in later from the agent")]
    pub no_login: bool,
    #[arg(
        long,
        alias = "no-browser",
        help = "Print the sign-in address instead of opening a browser"
    )]
    pub no_open: bool,
    #[arg(
        long,
        value_name = "DURATION",
        help = "Stop waiting for sign-in after this long (default: 10m)"
    )]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(
        long,
        help = "Exit 1 with an unhealthy_home error when the report finds problems"
    )]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct UnlinkArgs {
    #[arg(value_enum, value_name = "CLIENT", help = "Which agent to disconnect")]
    pub client: LinkClient,
    #[arg(
        long,
        value_enum,
        value_name = "SCOPE",
        default_value = "user",
        help = "Claude Code only: where the registration is saved"
    )]
    pub scope: LinkScope,
    #[arg(
        long,
        value_name = "NAME",
        default_value = "apple-bridge",
        help = "Name shown in the agent"
    )]
    pub name: String,
    #[arg(
        long,
        value_name = "DURATION",
        help = "Stop waiting for the agent's commands after this long (default: 1m)"
    )]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct StatsArgs {
    #[arg(
        short = 'f',
        long,
        conflicts_with_all = ["wait", "watch"],
        help = "Keep showing sync events until you stop it or --timeout passes"
    )]
    pub follow: bool,
    #[arg(
        short = 'w',
        long,
        conflicts_with = "watch",
        help = "Show sync events until every source has finished syncing, then exit"
    )]
    pub wait: bool,
    #[arg(long, hide = true)]
    pub watch: bool,
    #[arg(
        long,
        value_name = "DURATION",
        help = "With --follow or --wait: stop after this long, such as 30s or 5m; 0 checks once (default for --wait: 10m)"
    )]
    pub timeout: Option<String>,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "30s",
        help = "A source counts as finished after this long without new records; 0 means at once"
    )]
    pub settle: String,
    #[arg(short = 'a', long, help = "Include sources with nothing synced")]
    pub all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReadKind {
    Photos,
    Messages,
    Chats,
    Contacts,
    Calendar,
    Reminders,
    Notes,
    Mail,
    Voicemail,
}

impl ReadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Photos => "photos",
            Self::Messages => "messages",
            Self::Chats => "chats",
            Self::Contacts => "contacts",
            Self::Calendar => "calendar",
            Self::Reminders => "reminders",
            Self::Notes => "notes",
            Self::Mail => "mail",
            Self::Voicemail => "voicemail",
        }
    }
}

#[derive(Debug, Args)]
pub struct ReadArgs {
    #[arg(value_enum, value_name = "KIND", help = "What to read")]
    pub kind: ReadKind,
    #[arg(
        short = 'n',
        long,
        value_name = "NUMBER",
        allow_hyphen_values = true,
        help = "How many items to show in total, 1 to 200 (default: 10)"
    )]
    pub limit: Option<String>,
    #[arg(
        short = 'q',
        long,
        alias = "text",
        value_name = "TEXT",
        help = "Messages only: find messages containing this text"
    )]
    pub query: Option<String>,
    #[arg(long, value_name = "CHAT", help = "Messages only: read this chat")]
    pub chat: Option<String>,
    #[arg(
        long,
        value_name = "NUMBER",
        help = "Calendar only: how many days ahead (default: 7)"
    )]
    pub days: Option<String>,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "2m",
        help = "Stop fetching pages after this long; what was fetched is reported"
    )]
    pub timeout: String,
    #[arg(long, value_name = "CURSOR", help = "Continue from a previous page")]
    pub next: Option<String>,
    #[arg(short = 'a', long, help = "Fetch every page, up to 10,000 items")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(
        value_name = "TEXT",
        required_unless_present = "next",
        help = "Text to find; optional with --next"
    )]
    pub text: Option<String>,
    #[arg(
        short = 'n',
        long,
        value_name = "NUMBER",
        allow_hyphen_values = true,
        help = "How many results to show in total, 1 to 200 (default: 10)"
    )]
    pub limit: Option<String>,
    #[arg(long, value_name = "CURSOR", help = "Continue from a previous page")]
    pub next: Option<String>,
    #[arg(short = 'a', long, help = "Fetch every page, up to 10,000 results")]
    pub all: bool,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "2m",
        help = "Stop fetching pages after this long; what was fetched is reported"
    )]
    pub timeout: String,
}

#[derive(Debug, Args)]
pub struct CallArgs {
    #[arg(
        value_name = "OPERATION",
        help = "Operation name, such as sources.status"
    )]
    pub operation: Option<String>,
    #[arg(
        long,
        value_name = "JSON",
        help = "Arguments as JSON; - reads them from stdin (default: {})"
    )]
    pub data: Option<String>,
    #[arg(long, value_name = "JSON", hide = true)]
    pub args: Option<String>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Read the arguments from a JSON file; - means stdin"
    )]
    pub file: Option<PathBuf>,
    #[arg(long, help = "List read operations, and Photos writes with --write")]
    pub list: bool,
    #[arg(
        long,
        help = "Allow Photos organization writes; needs --token-env and a stable query ID"
    )]
    pub write: bool,
    #[arg(
        long,
        value_name = "NAME",
        help = "Environment variable holding an existing token to call with; it's never revoked"
    )]
    pub token_env: Option<String>,
}

#[derive(Debug, Args)]
pub struct SkillsCommand {
    #[command(subcommand)]
    pub action: Option<SkillsAction>,
}

#[derive(Debug, Subcommand)]
pub enum SkillsAction {
    #[command(about = "List the skills in this binary", visible_alias = "ls")]
    List,
    #[command(about = "Print one skill")]
    Get {
        #[arg(value_name = "NAME", help = "Skill name, such as core")]
        name: String,
    },
}

#[derive(Debug, Args)]
pub struct PermissionsCommand {
    #[command(subcommand)]
    pub action: Option<PermissionsAction>,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsAction {
    #[command(about = "Walk through the Mac permissions Apple Bridge reads with")]
    Guide(GuideArgs),
    #[command(about = "Show which permissions are allowed; never asks macOS")]
    Status(GuideStatusArgs),
    #[command(about = "Ask macOS for one permission through the Helper")]
    Request {
        #[arg(value_name = "PERMISSION", help = "Permission name, such as camera")]
        name: String,
        #[arg(long, value_name = "PATH", help = "Helper app to use")]
        helper_app: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct GuideArgs {
    #[arg(
        long,
        value_name = "NAMES",
        help = "Request only these comma-separated permissions, such as contacts,calendar"
    )]
    pub only: Option<String>,
    #[arg(long, value_name = "PATH", help = "Helper app to use")]
    pub helper_app: Option<String>,
    #[arg(
        long,
        help = "Walk through in this terminal instead of the guide window"
    )]
    pub in_terminal: bool,
    #[arg(long, value_name = "FILE", hide = true)]
    pub fixture: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct GuideStatusArgs {
    #[arg(long, value_name = "FILE", hide = true)]
    pub fixture: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct TailscaleArgs {
    #[arg(long, help = "Turn Tailscale access off again")]
    pub off: bool,
}

#[derive(Debug, Args)]
pub struct RemoteCommand {
    #[command(subcommand)]
    pub action: Option<RemoteAction>,
}

#[derive(Debug, Subcommand)]
pub enum RemoteAction {
    #[command(about = "Turn on remote access through the relay or a private route")]
    Enable(RemoteEnableArgs),
    #[command(about = "Turn off remote access")]
    Disable(RemoteDisableArgs),
    #[command(about = "Show remote access state")]
    Status(RemoteStatusArgs),
    #[command(about = "Print this Mac's relay key for a self-hosted relay")]
    Key(RemoteKeyArgs),
    #[command(about = "Manage certificates seen for this Mac's remote address")]
    Certificates(CertificatesCommand),
    #[command(
        hide = true,
        about = "Turn on remote access (deprecated: bridge remote enable)"
    )]
    On(RemoteEnableArgs),
    #[command(
        hide = true,
        about = "Turn off remote access (deprecated: bridge remote disable)"
    )]
    Off(RemoteDisableArgs),
    #[command(
        name = "certificate-accept",
        hide = true,
        about = "Accept a certificate (deprecated: bridge remote certificates accept)"
    )]
    CertificateAccept(FingerprintArgs),
}

#[derive(Debug, Args)]
pub struct RemoteEnableArgs {
    #[arg(
        long,
        value_name = "ROUTE",
        value_parser = route_values(),
        help = "Route (default: relay)"
    )]
    pub via: Option<String>,
    #[arg(long, help = "Use the private route; same as --via private")]
    pub private: bool,
    #[arg(long, value_name = "DOMAIN", help = "Custom domain on the relay route")]
    pub domain: Option<String>,
    #[arg(long, value_name = "URL", help = "Your own relay's address")]
    pub relay_url: Option<String>,
    #[arg(
        long,
        value_name = "SOURCES",
        help = "Cloud route only: services cloud chats may see, such as calendar,reminders"
    )]
    pub sources: Option<String>,
    #[arg(long, value_name = "NAME", help = "Name for this Mac")]
    pub name: Option<String>,
    #[arg(
        long,
        value_name = "DURATION",
        help = "Stop waiting after this long (default: 10m)"
    )]
    pub timeout: Option<String>,
    #[arg(
        long,
        conflicts_with = "no_open",
        help = "Open sign-in links in the browser even without a terminal, unless over SSH"
    )]
    pub open: bool,
    #[arg(long, help = "Print sign-in links instead of opening a browser")]
    pub no_open: bool,
}

fn route_values() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new([
        clap::builder::PossibleValue::new("relay"),
        clap::builder::PossibleValue::new("tailnet")
            .help("Your devices on your Tailscale network only"),
        clap::builder::PossibleValue::new("cloud")
            .help("A public address for cloud chats such as Claude and ChatGPT"),
        clap::builder::PossibleValue::new("private"),
        clap::builder::PossibleValue::new("tailscale-app"),
        clap::builder::PossibleValue::new("both"),
        clap::builder::PossibleValue::new("system").hide(true),
    ])
}

fn levels(switchable: bool) -> [clap::builder::PossibleValue; 3] {
    [
        clap::builder::PossibleValue::new("full").help("Acts at once"),
        clap::builder::PossibleValue::new("approve").help("Asks you before each change"),
        clap::builder::PossibleValue::new("read")
            .help("Reads only")
            .hide(switchable),
    ]
}

fn level_values(switchable: bool) -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(levels(switchable))
}

#[derive(Clone)]
pub struct LevelText(pub bool);

impl clap::builder::TypedValueParser for LevelText {
    type Value = String;

    fn parse_ref(
        &self,
        _command: &clap::Command,
        _argument: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<String, clap::Error> {
        Ok(value.to_string_lossy().into_owned())
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        Some(Box::new(levels(self.0).into_iter()))
    }
}

#[derive(Debug, Args)]
pub struct RemoteDisableArgs {
    #[arg(
        long,
        value_name = "ROUTE",
        value_parser = route_values(),
        help = "Route to turn off (default: relay)"
    )]
    pub via: Option<String>,
    #[arg(long, help = "Turn off the private route; same as --via private")]
    pub private: bool,
    #[arg(long, value_name = "NAME", help = "Name for this Mac")]
    pub name: Option<String>,
    #[arg(
        long,
        value_name = "DURATION",
        help = "Stop waiting after this long (default: 10m)"
    )]
    pub timeout: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoteStatusArgs {
    #[arg(long, help = "Print connection snippets for agents")]
    pub snippets: bool,
}

#[derive(Debug, Args)]
pub struct RemoteKeyArgs {
    #[arg(long, value_name = "URL", help = "Save this relay address first")]
    pub relay_url: Option<String>,
}

#[derive(Debug, Args)]
pub struct CertificatesCommand {
    #[command(subcommand)]
    pub action: Option<CertificatesAction>,
}

#[derive(Debug, Subcommand)]
pub enum CertificatesAction {
    #[command(about = "Accept a certificate you requested yourself")]
    Accept(FingerprintArgs),
}

#[derive(Debug, Args)]
pub struct FingerprintArgs {
    #[arg(
        value_name = "FINGERPRINT",
        help = "SHA-256 fingerprint: 64 hexadecimal characters"
    )]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConnectorsCommand {
    #[command(subcommand)]
    pub action: Option<ConnectorAction>,
}

#[derive(Debug, Subcommand)]
pub enum ConnectorAction {
    #[command(about = "List apps waiting for approval", visible_alias = "ls")]
    List(ListArgs),
    #[command(about = "Allow a waiting app after checking its code")]
    Allow(DecisionArgs),
    #[command(about = "Deny a waiting app")]
    Deny(DecisionArgs),
    #[command(
        hide = true,
        about = "List apps waiting for approval (deprecated: bridge connectors list)"
    )]
    Pending,
}

#[derive(Debug, Args)]
pub struct DecisionArgs {
    #[arg(
        long,
        value_name = "REQUEST",
        help = "Request ID from bridge connectors list (required)"
    )]
    pub request: Option<String>,
    #[arg(
        long,
        value_name = "CODE",
        help = "Four-digit code shown on the app's page (required)"
    )]
    pub code: Option<String>,
}

#[derive(Debug, Args)]
pub struct AgentsCommand {
    #[command(subcommand)]
    pub action: Option<AgentAction>,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    #[command(about = "List agents", visible_alias = "ls")]
    List(ListArgs),
    #[command(about = "Create an agent and print its token once")]
    Create(AgentCreateArgs),
    #[command(about = "Show one agent")]
    Inspect(AgentArgs),
    #[command(about = "Change an agent's level or access")]
    Update(AgentUpdateArgs),
    #[command(about = "Revoke an agent's token")]
    Revoke(AgentArgs),
    #[command(
        hide = true,
        about = "Change an agent's level (deprecated: bridge agents update --level)"
    )]
    Level(AgentLevelArgs),
}

#[derive(Debug, Args, Default)]
pub struct ListArgs {
    #[arg(
        short = 'n',
        long,
        value_name = "NUMBER",
        allow_hyphen_values = true,
        help = "How many to show, 1 to 200 (default: 50)"
    )]
    pub limit: Option<String>,
    #[arg(long, value_name = "CURSOR", help = "Continue from a previous page")]
    pub next: Option<String>,
}

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[arg(value_name = "AGENT", help = "Agent ID, such as agent:01J…")]
    pub id: String,
}

#[derive(Debug, Args)]
pub struct AgentLevelArgs {
    #[arg(value_name = "AGENT", help = "Agent ID")]
    pub id: String,
    #[arg(value_name = "LEVEL", value_parser = level_values(true), help = "New level")]
    pub level: String,
}

#[derive(Debug, Args)]
pub struct AgentCreateArgs {
    #[arg(long, value_name = "NAME", help = "Agent name, 1 to 120 bytes")]
    pub name: String,
    #[arg(
        long,
        value_name = "LEVEL",
        default_value = ab_trust::AgentLevel::DEFAULT_NAME,
        value_parser = LevelText(false),
        help = "What the agent may do"
    )]
    pub level: String,
    #[arg(
        long,
        value_name = "ACCESS",
        default_value = DEFAULT_AGENT_GRANTS,
        help = "Access grants, comma-separated, such as messages:read,contacts:read"
    )]
    pub scope: String,
    #[arg(
        long,
        value_name = "DURATION",
        help = "Stop the token working after this long, such as 1h, 7d or 90d (default: never)"
    )]
    pub expires_in: Option<String>,
    #[arg(
        long,
        help = "Let the token work only at this Mac's Tailscale address, from your other devices"
    )]
    pub tailnet: bool,
}

#[derive(Debug, Args)]
pub struct AgentUpdateArgs {
    #[arg(value_name = "AGENT", help = "Agent ID")]
    pub id: String,
    #[arg(long, value_name = "LEVEL", value_parser = LevelText(true), help = "New level")]
    pub level: Option<String>,
    #[arg(
        long,
        value_name = "ACCESS",
        help = "Replace its access grants, comma-separated"
    )]
    pub scope: Option<String>,
}

#[derive(Debug, Args)]
pub struct CredentialArgs {
    #[arg(value_name = "CREDENTIAL", help = "Agent, token or replica ID")]
    pub id: String,
}

#[derive(Debug, Args)]
pub struct ScopeArgs {
    #[arg(value_name = "CREDENTIAL", help = "Agent, token or replica ID")]
    pub id: String,
    #[arg(long, value_name = "ACCESS", default_value = DEFAULT_GRANTS, help = "Access grants, comma-separated")]
    pub scope: String,
}

#[derive(Debug, Args)]
pub struct TokensCommand {
    #[command(subcommand)]
    pub action: Option<TokenAction>,
}

#[derive(Debug, Subcommand)]
pub enum TokenAction {
    #[command(about = "Create a token and print it once")]
    Create {
        #[arg(long, value_name = "ACCESS", default_value = DEFAULT_GRANTS, help = "Access grants, comma-separated")]
        scope: String,
        #[arg(long, help = "Allow use through remote access")]
        remote: bool,
        #[arg(
            long,
            value_name = "DURATION",
            help = "Stop the token working after this long, such as 1h, 7d or 90d (default: never)"
        )]
        expires_in: Option<String>,
    },
    #[command(about = "Revoke a token, agent or replica")]
    Revoke(CredentialArgs),
    #[command(about = "Replace a token's, agent's or replica's access")]
    Update {
        #[arg(value_name = "CREDENTIAL", help = "Token, agent or replica ID")]
        id: String,
        #[arg(
            long,
            value_name = "ACCESS",
            required = true,
            help = "Access grants, comma-separated"
        )]
        scope: String,
    },
}

#[derive(Debug, Args)]
pub struct WebCommand {
    #[command(subcommand)]
    pub action: Option<WebAction>,
}

#[derive(Debug, Subcommand)]
pub enum WebAction {
    #[command(about = "Print a link and QR code to add a passkey")]
    Enroll {
        #[arg(
            long,
            value_name = "URL",
            help = "Page address: https://HOST, or http://localhost:PORT (default: this Mac's gateway address)"
        )]
        host: Option<String>,
        #[arg(long, help = "Use the remote access address")]
        public: bool,
    },
    #[command(about = "List or sign out signed-in browsers")]
    Sessions(SessionsCommand),
    #[command(
        hide = true,
        about = "Sign out every browser (deprecated: bridge web sessions revoke --all)"
    )]
    Lock,
    #[command(
        hide = true,
        about = "List signed-in browsers (deprecated: bridge web sessions list)"
    )]
    Devices,
}

#[derive(Debug, Args)]
pub struct SessionsCommand {
    #[command(subcommand)]
    pub action: Option<SessionsAction>,
}

#[derive(Debug, Subcommand)]
pub enum SessionsAction {
    #[command(about = "List signed-in browsers", visible_alias = "ls")]
    List(ListArgs),
    #[command(about = "Sign out browsers")]
    Revoke {
        #[arg(long, required = true, help = "Sign out every browser")]
        all: bool,
    },
}

#[derive(Debug, Args)]
pub struct StartArgs {
    #[arg(
        long,
        value_name = "ROLE",
        default_value = "gateway",
        value_parser = ["gateway", "replica"],
        help = "What this process runs as"
    )]
    pub role: String,
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "127.0.0.1:7780",
        help = "Local API address"
    )]
    pub api_addr: String,
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "",
        help = "Replica feed address"
    )]
    pub peer_addr: String,
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "",
        help = "Replica listen address"
    )]
    pub listen: String,
    #[arg(long, value_name = "PATH", default_value = "", help = "Helper socket")]
    pub socket: String,
    #[arg(long, value_name = "PATH", default_value = "", help = "Helper app")]
    pub helper_app: String,
    #[arg(long, hide = true)]
    pub helper_lab_commands: bool,
    #[arg(
        long,
        value_name = "NUMBER",
        default_value_t = 0,
        help = "Journal rows to keep"
    )]
    pub retention_rows: u64,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "0",
        help = "Journal age to keep"
    )]
    pub retention_age: String,
    #[arg(
        long,
        value_name = "NUMBER",
        default_value_t = 0,
        help = "Attachment storage limit in bytes"
    )]
    pub blob_cap: u64,
    #[arg(
        long,
        value_name = "NUMBER",
        default_value_t = 0,
        help = "Attachment removal grace in milliseconds"
    )]
    pub blob_grace_ms: u64,
}

#[derive(Debug, Args)]
pub struct PairArgs {
    #[arg(long, value_name = "ADDRESS", help = "Address the replica should dial")]
    pub peer_addr: Option<String>,
    #[arg(
        long,
        value_name = "ADDRESS",
        help = "Replica address the gateway should dial"
    )]
    pub dial_replica: Option<String>,
    #[arg(long, value_name = "ACCESS", default_value = DEFAULT_GRANTS, help = "Access grants for the replica, comma-separated")]
    pub scope: String,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "10m",
        help = "How long the code works"
    )]
    pub expires_in: String,
    #[arg(long, help = "Pair an iPhone; needs --via-remote")]
    pub iphone: bool,
    #[arg(long, help = "Pair through remote access")]
    pub via_remote: bool,
}

#[derive(Debug, Args)]
pub struct JoinArgs {
    #[arg(value_name = "CODE", help = "Pairing code from bridge pair")]
    pub code: String,
    #[arg(
        long,
        value_name = "ADDRESS",
        help = "Listen for the gateway on this address"
    )]
    pub listen: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConnectArgs {
    #[arg(value_name = "CODE", help = "Connection code")]
    pub code: String,
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "127.0.0.1:7780",
        help = "Local address to serve"
    )]
    pub listen: String,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[arg(value_name = "FILE", help = "File to write")]
    pub path: PathBuf,
    #[arg(long, help = "Replace the file if it exists")]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    #[arg(value_name = "FILE", help = "Backup file")]
    pub path: Option<PathBuf>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Backup file; same as the positional FILE"
    )]
    pub backup: Option<PathBuf>,
    #[arg(
        long,
        help = "Restore even though the command ledger is missing or doesn't match"
    )]
    pub accept_ledger_loss: bool,
}

#[derive(Debug, Args)]
pub struct ShortcutsCommand {
    #[command(subcommand)]
    pub action: Option<ShortcutsAction>,
}

#[derive(Debug, Subcommand)]
pub enum ShortcutsAction {
    #[command(about = "Open the Shortcuts pack for import")]
    Install(PackArgs),
    #[command(
        name = "install-pack",
        hide = true,
        about = "Open the Shortcuts pack (deprecated: bridge shortcuts install)"
    )]
    InstallPack(PackArgs),
}

#[derive(Debug, Args)]
pub struct PackArgs {
    #[arg(
        long,
        value_name = "DIR",
        default_value = ".build/shortcuts",
        help = "Pack folder"
    )]
    pub pack: PathBuf,
}

#[derive(Debug, Args)]
pub struct SelftestCommand {
    #[command(subcommand)]
    pub action: Option<SelftestAction>,
}

#[derive(Debug, Subcommand)]
pub enum SelftestAction {
    #[command(about = "Prove Messages operations by sending real messages to your own chat")]
    Messages(SelftestArgs),
}

#[derive(Debug, Args)]
pub struct SelftestArgs {
    #[arg(long, value_name = "PATH", help = "Helper app to use")]
    pub helper_app: Option<String>,
    #[arg(long, help = "Run again even if already proven")]
    pub force: bool,
    #[arg(long, help = "Skip steps that use the Messages window")]
    pub no_ui: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum, value_name = "SHELL", help = "bash, zsh or fish")]
    pub shell: Shell,
}

#[derive(Debug, Args)]
pub struct HelpArgs {
    #[arg(long, help = "List every command")]
    pub all: bool,
    #[arg(value_name = "COMMAND", help = "Command path, or search <TEXT>")]
    pub path: Vec<String>,
}

#[cfg(feature = "lab-harness")]
#[derive(Debug, Args)]
pub struct LabPrincipalArgs {
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub fingerprint: Option<String>,
}
