use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

const ABOUT: &str = "Connect local agents such as Claude Code and Codex to your Mac's Apple data.";
const QUICK_START: &str = "Quick start (no account, server or domain):
  bridge up                 start Apple Bridge on this Mac
  bridge permissions guide  allow the Mac permissions you want to use
  bridge link claude        connect Claude Code (or: bridge link codex)
  bridge stats              see how much is synced, counts only
  bridge skills get core    instructions for your agent

Remote use is separate and off until you ask: bridge remote on";

#[derive(Debug, Parser)]
#[command(
    name = "bridge",
    version,
    about = ABOUT,
    after_help = QUICK_START,
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        help = "Data folder [default: ~/Library/Application Support/AppleBridge]"
    )]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Start Apple Bridge on this Mac, for local agents. Asks nothing")]
    Up(UpArgs),
    #[command(about = "Stop a gateway that `bridge up` started")]
    Down,
    #[command(about = "Connect Claude Code or Codex to this Mac's local MCP address")]
    Link(LinkArgs),
    #[command(about = "Disconnect Claude Code or Codex")]
    Unlink(UnlinkArgs),
    #[command(about = "Show how much each source has synced. Counts only, never content")]
    Stats(StatsArgs),
    #[command(about = "Read synced data in the terminal")]
    Read(ReadArgs),
    #[command(about = "Search synced data in the terminal")]
    Search(SearchArgs),
    #[command(about = "Call any read operation by name, the same way an agent does")]
    Call(CallArgs),
    #[command(about = "Show agent instructions built into this binary")]
    Skills(SkillsCommand),
    #[command(about = "Check, guide or request Mac permissions")]
    Permissions(PermissionsCommand),
    #[command(about = "Show this home's role and sync position as JSON")]
    Status,
    #[command(about = "Check this home's data, files and credentials as JSON")]
    Doctor,
    #[command(about = "Remote use from other computers. Separate and off by default")]
    Remote(RemoteArgs),
    #[command(about = "Allow or deny apps waiting to connect")]
    Connector(ConnectorCommand),
    #[command(about = "Create, list or revoke agent access")]
    Agent(AgentCommand),
    #[command(about = "Run the gateway in the foreground. `bridge up` is usually easier")]
    Start(StartArgs),
    #[command(about = "Create a pairing code for a replica or iPhone")]
    Pair(PairArgs),
    #[command(about = "Join a gateway as a replica using a pairing code")]
    Join(JoinArgs),
    #[command(about = "Run the private replica client")]
    Connect(ConnectArgs),
    #[command(about = "Create a scoped API token")]
    Token(TokenCommand),
    #[command(about = "Revoke a credential by id")]
    Revoke(IdArgs),
    #[command(about = "Change a credential's access")]
    Scope(ScopeArgs),
    #[command(about = "Manage browser sign-in for this Mac's local pages")]
    Web(WebCommand),
    #[command(about = "Copy this home's data to a file")]
    Backup(BackupArgs),
    #[command(about = "Restore this home from a backup")]
    Restore(RestoreArgs),
    #[command(about = "Prepare credentials for the background service")]
    InstallCredentials,
    #[command(about = "Install the Shortcuts pack")]
    Shortcuts(ShortcutsCommand),
    #[command(about = "Run a Helper self-test that sends real messages to yourself")]
    Selftest(SelftestCommand),
    #[command(about = "Run the Messages self-test through the Helper")]
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
        value_name = "HOST:PORT",
        help = "Local API address [default: 127.0.0.1:7780]"
    )]
    pub api_addr: Option<String>,
    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Replica feed address [default: 127.0.0.1:7443, or any free port if that is taken]"
    )]
    pub peer_addr: Option<String>,
    #[arg(
        long,
        value_name = "APP",
        help = "Helper app [default: the app this binary ships in]"
    )]
    pub helper_app: Option<String>,
    #[arg(long, help = "Print the result as JSON")]
    pub json: bool,
    #[arg(
        long,
        help = "Return at once, even in a terminal, while syncing continues"
    )]
    pub no_watch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LinkClient {
    #[value(alias = "claude-code")]
    Claude,
    Codex,
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
    #[arg(value_enum, help = "Which agent to connect")]
    pub client: LinkClient,
    #[arg(
        long,
        value_enum,
        default_value = "user",
        help = "Claude Code only: where to save the connection"
    )]
    pub scope: LinkScope,
    #[arg(long, default_value = "apple-bridge", help = "Name shown in the agent")]
    pub name: String,
    #[arg(long, help = "Print the agent's commands instead of running them")]
    pub print: bool,
    #[arg(long, help = "Register only. Sign in later from the agent")]
    pub no_login: bool,
    #[arg(long, help = "Print the sign-in address instead of opening a browser")]
    pub no_browser: bool,
}

#[derive(Debug, Args)]
pub struct UnlinkArgs {
    #[arg(value_enum, help = "Which agent to disconnect")]
    pub client: LinkClient,
    #[arg(
        long,
        value_enum,
        default_value = "user",
        help = "Claude Code only: where the connection is saved"
    )]
    pub scope: LinkScope,
    #[arg(long, default_value = "apple-bridge", help = "Name shown in the agent")]
    pub name: String,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[arg(long, help = "Keep updating until every source has finished syncing")]
    pub watch: bool,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "Stop watching after this long [default: no limit]"
    )]
    pub timeout: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 30,
        help = "A source counts as finished after this long without new records"
    )]
    pub settle: u64,
    #[arg(long, help = "Include sources with nothing synced")]
    pub all: bool,
    #[arg(long, help = "Print JSON. With --watch, one JSON line per event")]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReadKind {
    Messages,
    Chats,
    Contacts,
    Calendar,
    Reminders,
    Notes,
    Mail,
}

#[derive(Debug, Args)]
pub struct ReadArgs {
    #[arg(value_enum, help = "What to read")]
    pub kind: ReadKind,
    #[arg(long, default_value_t = 10, help = "How many items to show")]
    pub limit: u32,
    #[arg(long, value_name = "TEXT", help = "Messages only: find this text")]
    pub text: Option<String>,
    #[arg(long, value_name = "ID", help = "Messages only: read this chat")]
    pub chat: Option<String>,
    #[arg(long, default_value_t = 7, help = "Calendar only: how many days ahead")]
    pub days: u32,
    #[arg(long, help = "Print the full JSON answer")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(help = "Text to find")]
    pub text: String,
    #[arg(long, default_value_t = 10, help = "How many results to show")]
    pub limit: u32,
    #[arg(long, help = "Print the full JSON answer")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CallArgs {
    #[arg(help = "Operation name, such as sources.status. Use --list to see them")]
    pub operation: Option<String>,
    #[arg(
        long,
        value_name = "JSON",
        default_value = "{}",
        help = "Arguments as JSON"
    )]
    pub args: String,
    #[arg(long, help = "List read operations")]
    pub list: bool,
}

#[derive(Debug, Args)]
pub struct SkillsCommand {
    #[command(subcommand)]
    pub action: SkillsAction,
}

#[derive(Debug, Subcommand)]
pub enum SkillsAction {
    #[command(about = "List the skills in this binary")]
    List,
    #[command(about = "Print one skill")]
    Get {
        #[arg(help = "Skill name, such as core")]
        name: String,
    },
}

#[derive(Debug, Args)]
pub struct PermissionsCommand {
    #[command(subcommand)]
    pub action: PermissionsAction,
}

#[derive(Debug, Subcommand)]
pub enum PermissionsAction {
    #[command(about = "Walk through the Mac permissions Apple Bridge reads with")]
    Guide(GuideArgs),
    #[command(about = "Show permission status. Never asks macOS for anything")]
    Status(GuideStatusArgs),
    #[command(about = "Ask macOS for one permission through the Helper")]
    Request {
        #[arg(help = "Permission name, such as camera")]
        name: String,
        #[arg(long, value_name = "APP", help = "Helper app to use")]
        helper_app: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct GuideArgs {
    #[arg(long, help = "Do each step without waiting for Enter")]
    pub yes: bool,
    #[arg(long, value_name = "APP", help = "Helper app to use")]
    pub helper_app: Option<String>,
    #[arg(long, value_name = "FILE", hide = true)]
    pub fixture: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct GuideStatusArgs {
    #[arg(long, help = "Print JSON")]
    pub json: bool,
    #[arg(long, value_name = "FILE", hide = true)]
    pub fixture: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RemoteAction {
    On,
    Off,
    Status,
    Key,
    CertificateAccept,
}

impl RemoteAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Status => "status",
            Self::Key => "key",
            Self::CertificateAccept => "certificate-accept",
        }
    }
}

#[derive(Debug, Args)]
pub struct RemoteArgs {
    #[arg(value_enum, help = "What to do")]
    pub action: RemoteAction,
    #[arg(help = "With certificate-accept: the certificate to accept")]
    pub value: Option<String>,
    #[arg(long, value_name = "ROUTE", help = "Route to use [default: relay]")]
    pub via: Option<String>,
    #[arg(long, help = "Use the private route")]
    pub private: bool,
    #[arg(long, help = "With status: print connection snippets")]
    pub snippets: bool,
    #[arg(long, value_name = "DOMAIN", help = "Custom relay domain")]
    pub domain: Option<String>,
    #[arg(long, value_name = "URL", help = "Your own relay's address")]
    pub relay_url: Option<String>,
    #[arg(long, value_name = "NAME", help = "Name for this Mac")]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConnectorCommand {
    #[command(subcommand)]
    pub action: ConnectorAction,
}

#[derive(Debug, Subcommand)]
pub enum ConnectorAction {
    #[command(about = "List apps waiting for approval")]
    Pending,
    #[command(about = "Allow a waiting app. Check the code matches first")]
    Allow(DecisionArgs),
    #[command(about = "Deny a waiting app")]
    Deny(DecisionArgs),
}

#[derive(Debug, Args)]
pub struct DecisionArgs {
    #[arg(long, value_name = "ID", help = "Request id from `connector pending`")]
    pub request: String,
    #[arg(long, value_name = "CODE", help = "Code shown on the app's page")]
    pub code: String,
}

#[derive(Debug, Args)]
pub struct AgentCommand {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    #[command(about = "List agents as JSON")]
    List,
    #[command(about = "Create an agent and print its token once")]
    Create(AgentCreateArgs),
    #[command(about = "Revoke an agent")]
    Revoke(IdArgs),
}

#[derive(Debug, Args)]
pub struct AgentCreateArgs {
    #[arg(long, value_name = "NAME", help = "Agent name")]
    pub name: String,
    #[arg(
        long,
        value_name = "LEVEL",
        default_value = "approve",
        help = "read, approve or full"
    )]
    pub level: String,
    #[arg(
        long,
        value_name = "SCOPES",
        default_value = "reminders:read,system:read",
        help = "Comma-separated access, such as messages:read,contacts:read"
    )]
    pub scope: String,
}

#[derive(Debug, Args)]
pub struct IdArgs {
    #[arg(help = "Credential id")]
    pub id: String,
}

#[derive(Debug, Args)]
pub struct ScopeArgs {
    #[arg(help = "Credential id")]
    pub id: String,
    #[arg(
        long,
        value_name = "SCOPES",
        default_value = "reminders:read,system:read",
        help = "Comma-separated access, such as messages:read,contacts:read"
    )]
    pub scope: String,
}

#[derive(Debug, Args)]
pub struct StartArgs {
    #[arg(long, default_value = "gateway", help = "gateway or replica")]
    pub role: String,
    #[arg(
        long,
        value_name = "HOST:PORT",
        default_value = "127.0.0.1:7780",
        help = "Local API address"
    )]
    pub api_addr: String,
    #[arg(
        long,
        value_name = "HOST:PORT",
        default_value = "",
        help = "Replica feed address"
    )]
    pub peer_addr: String,
    #[arg(
        long,
        value_name = "HOST:PORT",
        default_value = "",
        help = "Replica listen address"
    )]
    pub listen: String,
    #[arg(long, value_name = "PATH", default_value = "", help = "Helper socket")]
    pub socket: String,
    #[arg(long, value_name = "APP", default_value = "", help = "Helper app")]
    pub helper_app: String,
    #[arg(long, hide = true)]
    pub helper_lab_commands: bool,
    #[arg(
        long,
        value_name = "N",
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
        value_name = "BYTES",
        default_value_t = 0,
        help = "Attachment storage limit"
    )]
    pub blob_cap: u64,
    #[arg(
        long,
        value_name = "MS",
        default_value_t = 0,
        help = "Attachment removal grace"
    )]
    pub blob_grace_ms: u64,
}

#[derive(Debug, Args)]
pub struct PairArgs {
    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Address the replica should dial"
    )]
    pub peer_addr: Option<String>,
    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Replica address the gateway should dial"
    )]
    pub dial_replica: Option<String>,
    #[arg(
        long,
        value_name = "SCOPES",
        default_value = "reminders:read,system:read",
        help = "Comma-separated access"
    )]
    pub scope: String,
    #[arg(
        long,
        value_name = "DURATION",
        default_value = "10m",
        help = "How long the code works"
    )]
    pub expires_in: String,
    #[arg(long, help = "Pair an iPhone. Needs --via-remote")]
    pub iphone: bool,
    #[arg(long, help = "Pair through remote access")]
    pub via_remote: bool,
}

#[derive(Debug, Args)]
pub struct JoinArgs {
    #[arg(help = "Pairing code")]
    pub code: String,
    #[arg(
        long,
        value_name = "HOST:PORT",
        help = "Listen for the gateway on this address"
    )]
    pub listen: Option<String>,
}

#[derive(Debug, Args)]
pub struct ConnectArgs {
    #[arg(help = "Connection code")]
    pub code: String,
    #[arg(
        long,
        value_name = "HOST:PORT",
        default_value = "127.0.0.1:7780",
        help = "Local address to serve"
    )]
    pub listen: String,
}

#[derive(Debug, Args)]
pub struct TokenCommand {
    #[command(subcommand)]
    pub action: TokenAction,
}

#[derive(Debug, Subcommand)]
pub enum TokenAction {
    #[command(about = "Create a token and print it once")]
    Create {
        #[arg(
            long,
            value_name = "SCOPES",
            default_value = "reminders:read,system:read",
            help = "Comma-separated access"
        )]
        scope: String,
        #[arg(long, help = "Allow use through remote access")]
        remote: bool,
    },
}

#[derive(Debug, Args)]
pub struct WebCommand {
    #[command(subcommand)]
    pub action: WebAction,
}

#[derive(Debug, Subcommand)]
pub enum WebAction {
    #[command(about = "Print a link and QR code to add a passkey")]
    Enroll {
        #[arg(
            long,
            value_name = "URL",
            default_value = "http://localhost:7780",
            help = "Page address"
        )]
        host: String,
        #[arg(long, help = "Use the remote address")]
        public: bool,
    },
    #[command(about = "Sign out every browser")]
    Lock,
    #[command(about = "List signed-in browsers as JSON")]
    Devices,
}

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[arg(help = "File to write")]
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    #[arg(help = "Backup file")]
    pub path: Option<PathBuf>,
    #[arg(long, value_name = "FILE", help = "Backup file")]
    pub backup: Option<PathBuf>,
    #[arg(long, help = "Restore even though the command ledger is missing")]
    pub accept_ledger_loss: bool,
}

#[derive(Debug, Args)]
pub struct ShortcutsCommand {
    #[command(subcommand)]
    pub action: ShortcutsAction,
}

#[derive(Debug, Subcommand)]
pub enum ShortcutsAction {
    #[command(name = "install-pack", about = "Open the Shortcuts pack for import")]
    InstallPack {
        #[arg(
            long,
            value_name = "DIR",
            default_value = ".build/shortcuts",
            help = "Pack folder"
        )]
        pack: PathBuf,
    },
}

#[derive(Debug, Args)]
pub struct SelftestCommand {
    #[command(subcommand)]
    pub action: SelftestAction,
}

#[derive(Debug, Subcommand)]
pub enum SelftestAction {
    #[command(about = "Prove Messages operations on your own chat")]
    Messages(SelftestArgs),
}

#[derive(Debug, Args)]
pub struct SelftestArgs {
    #[arg(long, value_name = "APP", help = "Helper app to use")]
    pub helper_app: Option<String>,
    #[arg(long, help = "Run again even if already proven")]
    pub force: bool,
    #[arg(long, help = "Skip steps that use the Messages window")]
    pub no_ui: bool,
}

#[cfg(feature = "lab-harness")]
#[derive(Debug, Args)]
pub struct LabPrincipalArgs {
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub fingerprint: Option<String>,
}
