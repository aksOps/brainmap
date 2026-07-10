use crate::{
    bench, context, eval, export, gate, harness, index, install, learning, mcp, model, skill,
    snapshot, util, vault, web,
};
use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "brainmap",
    version,
    about = "Local deterministic personal decision engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init(InitArgs),
    #[command(name = "init-vault")]
    InitVault(InitVaultArgs),
    Status(VaultArg),
    Doctor(VaultArg),
    #[command(name = "build-decision-engine")]
    BuildDecisionEngine(BuildArgs),
    #[command(name = "build-brain")]
    BuildBrain(BuildArgs),
    Gate(GateArgs),
    Context(ContextArgs),
    #[command(name = "should-ask-user")]
    ShouldAskUser(ShouldAskArgs),
    Decide(DecideArgs),
    #[command(name = "record-decision")]
    RecordDecision(RecordDecisionArgs),
    #[command(name = "learn-feedback")]
    LearnFeedback(LearnFeedbackArgs),
    #[command(name = "learn-decision")]
    LearnDecision(LearnDecisionArgs),
    Calibrate(CalibrateArgs),
    Autopilot(AutopilotArgs),
    #[command(name = "gate-mode")]
    GateMode(GateModeArgs),
    Capture(CaptureArgs),
    Extract(ExtractArgs),
    Apply(ApplyArgs),
    #[command(name = "prune-imports")]
    PruneImports(PruneImportsArgs),
    #[command(name = "review-decisions")]
    ReviewDecisions(ReviewArgs),
    Dream(DreamArgs),
    Index(IndexArgs),
    #[command(name = "link-check")]
    LinkCheck(VaultArg),
    Graph(GraphArgs),
    Search(SearchArgs),
    Embed(EmbedArgs),
    Models(ModelsArgs),
    Export(ExportArgs),
    Import(ImportArgs),
    Restore(RestoreArgs),
    #[command(name = "verify-export")]
    VerifyExport(VerifyExportArgs),
    Web(WebArgs),
    Mcp(McpArgs),
    Harness(HarnessArgs),
    Skill(SkillArgs),
    Install(InstallArgs),
    Snapshot(SnapshotArgs),
    Rollback(RollbackArgs),
    Bench(BenchArgs),
    Eval(EvalArgs),
}

#[derive(Args, Clone)]
pub struct VaultArg {
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct InitVaultArgs {
    #[arg(long)]
    vault: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Args, Clone)]
pub struct BuildArgs {
    #[arg(long, default_value = "auto")]
    pub mode: String,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long, default_value_t = 7)]
    pub questions: usize,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub file: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct GateArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long, default_value = "unknown")]
    pub intent: String,
    #[arg(long, default_value = "")]
    pub situation: String,
    #[arg(long, default_value = "")]
    pub options: String,
    #[arg(long, default_value = "")]
    pub proposed_action: String,
    #[arg(long, default_value = "medium")]
    pub risk: String,
    #[arg(long)]
    pub reversible: Option<bool>,
    #[arg(long, default_value = "general")]
    pub decision_type: String,
    #[arg(long, default_value = "global")]
    pub scope: String,
    #[arg(long)]
    pub agent_confidence: Option<f64>,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Clone)]
pub struct ShouldAskArgs {
    #[arg(long, default_value = "")]
    pub question: String,
    #[arg(long, default_value = "")]
    pub situation: String,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct ContextArgs {
    #[arg(long)]
    pub fast: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long, default_value_t = 8)]
    pub limit: usize,
}

#[derive(Args, Clone)]
pub struct DecideArgs {
    pub situation: Option<String>,
    #[arg(long, default_value = "")]
    pub options: String,
    #[arg(long, default_value = "medium")]
    pub risk: String,
    #[arg(long)]
    pub reversible: Option<bool>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct RecordDecisionArgs {
    #[arg(long)]
    pub decision_id: Option<String>,
    #[arg(long)]
    pub chosen: Option<String>,
    #[arg(long)]
    pub was_asked: Option<bool>,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct LearnFeedbackArgs {
    #[arg(long)]
    pub decision_id: String,
    #[arg(long)]
    pub correction: String,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct LearnDecisionArgs {
    #[arg(long)]
    pub situation: String,
    #[arg(long, default_value = "")]
    pub options: String,
    #[arg(long)]
    pub chosen: String,
    #[arg(long)]
    pub rejected: Option<String>,
    #[arg(long)]
    pub rationale: Option<String>,
    #[arg(long, default_value = "general")]
    pub decision_type: String,
    #[arg(long, default_value = "global")]
    pub scope: String,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct CalibrateArgs {
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long, default_value_t = 7)]
    pub n: usize,
    #[arg(long, default_value = "all")]
    pub topic: String,
}

#[derive(Args)]
struct AutopilotArgs {
    #[command(subcommand)]
    command: AutopilotCommand,
}

#[derive(Subcommand)]
enum AutopilotCommand {
    Status(VaultArg),
    Enable {
        #[arg(long, default_value = "conservative")]
        level: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    Disable(VaultArg),
    Promote {
        #[arg(long)]
        to: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    Demote {
        #[arg(long)]
        to: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    #[command(name = "set-threshold")]
    SetThreshold {
        #[arg(long)]
        confidence: f64,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
}

#[derive(Args)]
struct GateModeArgs {
    mode: String,
    #[arg(long)]
    vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct CaptureArgs {
    #[arg(long)]
    pub stdin: bool,
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long, default_value = "manual")]
    pub source: String,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct ExtractArgs {
    #[arg(long)]
    pub from_queue: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
pub struct ApplyArgs {
    #[arg(long)]
    pub pending: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub struct PruneImportsArgs {
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
struct ReviewArgs {
    cadence: String,
    #[arg(long)]
    vault: Option<PathBuf>,
}

#[derive(Args)]
struct DreamArgs {
    #[arg(long, default_value = "lite")]
    mode: String,
    #[arg(long)]
    vault: Option<PathBuf>,
}

#[derive(Args)]
struct IndexArgs {
    #[command(subcommand)]
    command: IndexCommand,
}

#[derive(Subcommand)]
enum IndexCommand {
    Rebuild(VaultArg),
    Status(VaultArg),
    Verify(VaultArg),
}

#[derive(Args)]
struct GraphArgs {
    #[command(subcommand)]
    command: GraphCommand,
}

#[derive(Subcommand)]
enum GraphCommand {
    Neighbors {
        id: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    Path {
        from: String,
        to: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    Orphans(VaultArg),
}

#[derive(Args)]
pub struct SearchArgs {
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub vector: Option<String>,
    #[arg(long)]
    pub hybrid: Option<String>,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Args)]
struct EmbedArgs {
    #[command(subcommand)]
    command: EmbedCommand,
}

#[derive(Subcommand)]
enum EmbedCommand {
    Rebuild(VaultArg),
    Process {
        #[arg(long)]
        missing_only: bool,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    Status(VaultArg),
}

#[derive(Args)]
struct ModelsArgs {
    #[command(subcommand)]
    command: ModelsCommand,
}

#[derive(Subcommand)]
enum ModelsCommand {
    Status(VaultArg),
    Materialize {
        #[arg(long)]
        vault: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    Verify(VaultArg),
    Info,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum ExportMode {
    Portable,
    Full,
    ShareSafe,
    Encrypted,
}

#[derive(Args)]
pub struct ExportArgs {
    #[arg(long, value_enum, default_value_t = ExportMode::Portable)]
    pub mode: ExportMode,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub out: PathBuf,
    #[arg(long)]
    pub encrypt: bool,
    #[arg(long)]
    pub recipient: Option<String>,
}

#[derive(Args)]
pub struct ImportArgs {
    #[arg(long)]
    pub file: PathBuf,
    #[arg(long)]
    pub to: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub identity: Option<PathBuf>,
}

#[derive(Args)]
pub struct RestoreArgs {
    #[arg(long)]
    pub file: PathBuf,
    #[arg(long)]
    pub to: PathBuf,
    #[arg(long)]
    pub identity: Option<PathBuf>,
}

#[derive(Args)]
pub struct VerifyExportArgs {
    pub file: PathBuf,
    #[arg(long)]
    pub identity: Option<PathBuf>,
}

#[derive(Args)]
struct WebArgs {
    #[command(subcommand)]
    command: Option<WebCommand>,
    #[arg(long)]
    vault: Option<PathBuf>,
    #[arg(long)]
    open: bool,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8777)]
    port: u16,
}

#[derive(Subcommand)]
enum WebCommand {
    #[command(name = "export-static")]
    ExportStatic {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
}

#[derive(Args)]
struct McpArgs {
    #[command(subcommand)]
    command: McpCommand,
}

#[derive(Subcommand)]
enum McpCommand {
    Serve {
        #[arg(long)]
        vault: Option<PathBuf>,
    },
}

#[derive(Args)]
struct HarnessArgs {
    #[command(subcommand)]
    command: HarnessCommand,
}

#[derive(Subcommand)]
enum HarnessCommand {
    Stdio(harness::StdioArgs),
    Hook(harness::HookArgs),
}

#[derive(Args)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCommand,
}

#[derive(Subcommand)]
enum SkillCommand {
    #[command(name = "build-decision-engine")]
    BuildDecisionEngine(skill::BuildDecisionEngineSkillArgs),
}

#[derive(Args)]
struct InstallArgs {
    #[command(subcommand)]
    command: InstallCommand,
}

#[derive(Subcommand)]
enum InstallCommand {
    Harness(install::InstallHarnessArgs),
}

#[derive(Args)]
struct SnapshotArgs {
    #[command(subcommand)]
    command: SnapshotCommand,
}

#[derive(Subcommand)]
enum SnapshotCommand {
    Create(VaultArg),
    List(VaultArg),
    Restore {
        id: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
}

#[derive(Args)]
struct RollbackArgs {
    #[command(subcommand)]
    command: RollbackCommand,
}

#[derive(Subcommand)]
enum RollbackCommand {
    Last(VaultArg),
}

#[derive(Args)]
pub struct BenchArgs {
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub scale: Option<usize>,
    #[arg(long)]
    pub embeddings: bool,
    #[arg(long, default_value = "local first decisions")]
    pub query: String,
}

#[derive(Args)]
pub struct EvalArgs {
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub suite: PathBuf,
}

pub fn run() -> Result<()> {
    let args: Vec<OsString> = util::strip_optional_program_alias(std::env::args_os().collect());
    let cli = Cli::parse_from(args);
    match cli.command {
        Command::Init(args) => vault::init_config(args.dry_run),
        Command::InitVault(args) => vault::init_vault(args.vault, args.dry_run, args.yes),
        Command::Status(args) => vault::status(args.vault),
        Command::Doctor(args) => vault::doctor(args.vault),
        Command::BuildDecisionEngine(args) | Command::BuildBrain(args) => {
            learning::build_decision_engine(args)
        }
        Command::Gate(args) => gate::cmd_gate(args),
        Command::Context(args) => context::cmd_context(args),
        Command::ShouldAskUser(args) => gate::cmd_should_ask(args),
        Command::Decide(args) => gate::cmd_decide(args),
        Command::RecordDecision(args) => learning::record_decision(args),
        Command::LearnFeedback(args) => learning::learn_feedback(args),
        Command::LearnDecision(args) => learning::learn_decision(args),
        Command::Calibrate(args) => learning::calibrate(args),
        Command::Autopilot(args) => match args.command {
            AutopilotCommand::Status(v) => learning::autopilot_status(v.vault),
            AutopilotCommand::Enable { level, vault } => {
                learning::autopilot_set(vault, "shadow", &level, None)
            }
            AutopilotCommand::Disable(v) => {
                learning::autopilot_set(v.vault, "disabled", "off", None)
            }
            AutopilotCommand::Promote { to, vault } => learning::autopilot_promote(vault, &to),
            AutopilotCommand::Demote { to, vault } => {
                learning::autopilot_set(vault, &to, "conservative", None)
            }
            AutopilotCommand::SetThreshold { confidence, vault } => {
                learning::autopilot_set_threshold(vault, confidence)
            }
        },
        Command::GateMode(args) => learning::gate_mode(args.vault, &args.mode),
        Command::Capture(args) => learning::capture(args),
        Command::Extract(args) => learning::extract(args),
        Command::Apply(args) => learning::apply(args),
        Command::PruneImports(args) => learning::prune_imports(args),
        Command::ReviewDecisions(args) => learning::review(args.vault, &args.cadence),
        Command::Dream(args) => learning::dream(args.vault, &args.mode),
        Command::Index(args) => match args.command {
            IndexCommand::Rebuild(v) => index::rebuild_cmd(v.vault),
            IndexCommand::Status(v) => index::status_cmd(v.vault),
            IndexCommand::Verify(v) => index::verify_cmd(v.vault),
        },
        Command::LinkCheck(args) => vault::link_check_cmd(args.vault),
        Command::Graph(args) => match args.command {
            GraphCommand::Neighbors { id, vault } => index::graph_neighbors_cmd(vault, &id),
            GraphCommand::Path { from, to, vault } => index::graph_path_cmd(vault, &from, &to),
            GraphCommand::Orphans(v) => index::graph_orphans_cmd(v.vault),
        },
        Command::Search(args) => index::search_cmd(args),
        Command::Embed(args) => match args.command {
            EmbedCommand::Rebuild(v) => model::embed_rebuild(v.vault),
            EmbedCommand::Process {
                missing_only,
                vault,
            } => model::embed_process(vault, missing_only),
            EmbedCommand::Status(v) => model::embed_status(v.vault),
        },
        Command::Models(args) => match args.command {
            ModelsCommand::Status(v) => model::models_status(v.vault),
            ModelsCommand::Materialize { vault, force } => model::models_materialize(vault, force),
            ModelsCommand::Verify(v) => model::models_verify(v.vault),
            ModelsCommand::Info => model::models_info(),
        },
        Command::Export(args) => export::export_cmd(args),
        Command::Import(args) => export::import_cmd(args),
        Command::Restore(args) => export::restore_cmd(args),
        Command::VerifyExport(args) => export::verify_export_cmd(args),
        Command::Web(args) => match args.command {
            Some(WebCommand::ExportStatic { out, vault }) => {
                web::export_static(vault.or(args.vault), out)
            }
            None => web::serve(args.vault, &args.host, args.port, args.open),
        },
        Command::Mcp(args) => match args.command {
            McpCommand::Serve { vault } => mcp::serve(vault),
        },
        Command::Harness(args) => match args.command {
            HarnessCommand::Stdio(args) => harness::stdio(args),
            HarnessCommand::Hook(args) => harness::hook(args),
        },
        Command::Skill(args) => match args.command {
            SkillCommand::BuildDecisionEngine(args) => skill::build_decision_engine_cmd(args),
        },
        Command::Install(args) => match args.command {
            InstallCommand::Harness(args) => install::install_harness(args),
        },
        Command::Snapshot(args) => match args.command {
            SnapshotCommand::Create(v) => snapshot::create(v.vault),
            SnapshotCommand::List(v) => snapshot::list(v.vault),
            SnapshotCommand::Restore { id, vault } => snapshot::restore(vault, &id),
        },
        Command::Rollback(args) => match args.command {
            RollbackCommand::Last(v) => snapshot::rollback_last(v.vault),
        },
        Command::Bench(args) => bench::run(args),
        Command::Eval(args) => eval::run(args),
    }
}
