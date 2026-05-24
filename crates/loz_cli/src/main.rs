use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use loz_ast::{
    AgentDeclaration, Diagnostic, Expression, ExpressionKind, FunctionParameter, Program,
    Statement, TypeName, WorkflowDeclaration, WorkflowTarget,
};
use loz_codegen::{Interpreter, RuntimeValue, WorkflowStepOutcome, execute, generate_llvm_ir};
use loz_lexer::tokenize_with_file_path;
use loz_optimizer::optimize_program;
use loz_parser::parse_program;
use loz_semantic::analyze;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value as JsonValue;

static BUILD_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);
const BUILTIN_MODULES: &[&str] = &["io", "json", "schema", "python", "llm"];

fn main() {
    if let Err(error) = run_cli() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_cli() -> Result<(), CliError> {
    let invocation = parse_cli_args(std::env::args())?;
    let current_dir = env::current_dir()
        .map_err(|error| CliError::new(format!("failed to read current directory: {error}")))?;

    match invocation.command {
        CliCommand::Program(program_command) => {
            let context =
                resolve_project_context(program_command.source_argument.as_deref(), &current_dir)?;
            context.apply_runtime_environment()?;

            let _working_directory = WorkingDirectoryGuard::change_to(&context.project_root)?;
            let program = load_checked_program(&context.source_path)?;
            let program = optimize_checked_program(&program, &context.source_path)?;

            match program_command.kind {
                ProgramCommandKind::Run => {
                    execute(&program).map_err(|error| CliError::new(error.to_string()))?;
                    Ok(())
                }
                ProgramCommandKind::Check => {
                    println!("Check passed.");
                    Ok(())
                }
                ProgramCommandKind::LlvmIr => {
                    let ir = generate_llvm_ir(&program).map_err(|error| {
                        CliError::with_file_path(error.to_string(), &context.source_path)
                    })?;
                    println!("{ir}");
                    Ok(())
                }
                ProgramCommandKind::Build => {
                    build_native_executable(&context.source_path, &context.project_root, &program)
                }
            }
        }
        CliCommand::Agent(agent_command) => run_agent_cli(agent_command, &current_dir),
        CliCommand::Workflow(workflow_command) => run_workflow_cli(workflow_command, &current_dir),
        CliCommand::Deps => run_deps(&current_dir),
        CliCommand::Doctor => run_doctor(&current_dir),
        CliCommand::Init { project_path } => run_init(&current_dir, &project_path),
        CliCommand::Help => {
            println!("{}", cli_usage());
            Ok(())
        }
        CliCommand::Version => {
            println!("{}", cli_version());
            Ok(())
        }
    }
}

fn optimize_checked_program(program: &Program, source_path: &Path) -> Result<Program, CliError> {
    optimize_program(program)
        .map_err(|error| CliError::with_file_path(error.to_string(), source_path))
}

fn load_checked_program(source_path: &Path) -> Result<Program, CliError> {
    let program = load_program_from_entry(source_path)?;
    analyze(&program).map_err(|error| CliError::from_diagnostic(error.diagnostic.clone()))?;
    Ok(program)
}

fn parse_cli_args<I>(args: I) -> Result<CliInvocation, CliError>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _binary_name = args.next();

    let command = args.next().ok_or_else(|| CliError::new(cli_usage()))?;

    let command = match command.as_str() {
        "run" | "check" | "llvm-ir" | "build" => {
            let source_argument = args.next().map(PathBuf::from);
            if args.next().is_some() {
                return Err(CliError::new(cli_usage()));
            }

            CliCommand::Program(ProgramCommandInvocation {
                kind: ProgramCommandKind::parse(&command)?,
                source_argument,
            })
        }
        "deps" => {
            if args.next().is_some() {
                return Err(CliError::new("usage: loz deps"));
            }
            CliCommand::Deps
        }
        "doctor" => {
            if args.next().is_some() {
                return Err(CliError::new("usage: loz doctor"));
            }
            CliCommand::Doctor
        }
        "init" => {
            let project_path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| CliError::new("usage: loz init <project-name>"))?;
            if args.next().is_some() {
                return Err(CliError::new("usage: loz init <project-name>"));
            }
            CliCommand::Init { project_path }
        }
        "agent" => {
            let subcommand = args.next().ok_or_else(|| CliError::new(agent_usage()))?;
            CliCommand::Agent(AgentCommandInvocation::parse(&subcommand, args.collect())?)
        }
        "workflow" => {
            let subcommand = args.next().ok_or_else(|| CliError::new(workflow_usage()))?;
            CliCommand::Workflow(WorkflowCommandInvocation::parse(
                &subcommand,
                args.collect(),
            )?)
        }
        "--help" | "-h" => {
            if args.next().is_some() {
                return Err(CliError::new(cli_usage()));
            }
            CliCommand::Help
        }
        "--version" | "-V" => {
            if args.next().is_some() {
                return Err(CliError::new("usage: loz --version"));
            }
            CliCommand::Version
        }
        other => {
            return Err(CliError::new(format!(
                "unknown command '{other}'. usage: {}",
                cli_usage()
            )));
        }
    };

    Ok(CliInvocation { command })
}

fn resolve_project_context(
    source_argument: Option<&Path>,
    current_dir: &Path,
) -> Result<ProjectContext, CliError> {
    match source_argument {
        Some(source_argument) => {
            let source_path = resolve_path_from_cwd(current_dir, source_argument);
            let project_root = find_project_root_from_source(&source_path)
                .unwrap_or_else(|| current_dir.to_path_buf());
            let project_root = canonicalize_existing_dir(&project_root)?;
            let config = load_project_file_config(&project_root)?;
            let dotenv_values = load_dotenv_values(&project_root)?;

            Ok(ProjectContext {
                project_root,
                source_path: canonicalize_existing_file(&source_path)?,
                config,
                dotenv_values,
            })
        }
        None => {
            let project_root = find_project_root_from_dir(current_dir).ok_or_else(|| {
                CliError::new("no source file provided and no loz.toml with [project].main found")
            })?;
            let project_root = canonicalize_existing_dir(&project_root)?;
            let config = load_required_project_file_config(&project_root)?;
            let main_source = config
                .project
                .as_ref()
                .and_then(|project| project.main.as_ref())
                .ok_or_else(|| {
                    CliError::new(
                        "no source file provided and no loz.toml with [project].main found",
                    )
                })?;
            let main_source_path = resolve_project_relative_path(&project_root, main_source);
            if !main_source_path.is_file() {
                return Err(CliError::new(format!(
                    "source path from loz.toml [project].main does not exist: '{}'",
                    main_source_path.display()
                )));
            }

            let dotenv_values = load_dotenv_values(&project_root)?;
            Ok(ProjectContext {
                project_root,
                source_path: canonicalize_existing_file(&main_source_path)?,
                config,
                dotenv_values,
            })
        }
    }
}

fn run_agent_cli(
    agent_command: AgentCommandInvocation,
    current_dir: &Path,
) -> Result<(), CliError> {
    let (source_argument, agent_behavior) = resolve_agent_command(agent_command, current_dir)?;
    let context = resolve_project_context(source_argument.as_deref(), current_dir)?;
    context.apply_runtime_environment()?;

    let _working_directory = WorkingDirectoryGuard::change_to(&context.project_root)?;
    let program = load_checked_program(&context.source_path)?;
    let program = optimize_checked_program(&program, &context.source_path)?;

    let output = match agent_behavior {
        ResolvedAgentCommand::List => render_agent_list(&collect_agents(&program)),
        ResolvedAgentCommand::Run { arguments } => {
            run_agent_task(&program, &collect_agents(&program), &arguments)?
        }
    };

    if !output.is_empty() {
        println!("{output}");
    }

    Ok(())
}

fn cli_usage() -> &'static str {
    "Usage:
  loz run [source.loz]
  loz check [source.loz]
  loz llvm-ir [source.loz]
  loz build [source.loz]
  loz deps
  loz agent list [source.loz]
  loz agent run [source.loz] [AgentName] [TaskName] [args...]
  loz workflow list [source.loz]
  loz workflow run [source.loz] [WorkflowName]
  loz doctor
  loz init <project-name>
  loz --version"
}

fn cli_version() -> String {
    format!("loz {}", env!("CARGO_PKG_VERSION"))
}

fn agent_usage() -> &'static str {
    "loz agent <list|run> ..."
}

fn agent_list_usage() -> &'static str {
    "loz agent list [source.loz]"
}

fn workflow_usage() -> &'static str {
    "loz workflow <list|run> ..."
}

fn workflow_list_usage() -> &'static str {
    "loz workflow list [source.loz]"
}

#[derive(Debug, PartialEq, Eq)]
enum ResolvedAgentCommand {
    List,
    Run { arguments: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
struct DiscoveredAgent {
    name: String,
    model: Option<String>,
    tools: Vec<String>,
    tasks: Vec<DiscoveredTask>,
}

#[derive(Debug, Clone, PartialEq)]
struct DiscoveredTask {
    name: String,
    parameters: Vec<FunctionParameter>,
    return_type: TypeName,
}

#[derive(Debug, Clone, PartialEq)]
struct DiscoveredWorkflow {
    name: String,
    steps: Vec<DiscoveredWorkflowStep>,
}

#[derive(Debug, Clone, PartialEq)]
struct DiscoveredWorkflowStep {
    name: String,
    target: WorkflowTarget,
}

#[derive(Debug)]
struct AgentTaskSelection<'a> {
    agent: &'a DiscoveredAgent,
    task: &'a DiscoveredTask,
    arguments: &'a [String],
}

fn resolve_agent_command(
    agent_command: AgentCommandInvocation,
    current_dir: &Path,
) -> Result<(Option<PathBuf>, ResolvedAgentCommand), CliError> {
    match agent_command {
        AgentCommandInvocation::List { source_argument } => {
            Ok((source_argument, ResolvedAgentCommand::List))
        }
        AgentCommandInvocation::Run { raw_arguments } => {
            let (source_argument, consumed_arguments) =
                split_agent_run_source_argument(&raw_arguments, current_dir);
            Ok((
                source_argument,
                ResolvedAgentCommand::Run {
                    arguments: raw_arguments[consumed_arguments..].to_vec(),
                },
            ))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ResolvedWorkflowCommand {
    List,
    Run { workflow_name: Option<String> },
}

fn run_workflow_cli(
    workflow_command: WorkflowCommandInvocation,
    current_dir: &Path,
) -> Result<(), CliError> {
    let (source_argument, workflow_behavior) =
        resolve_workflow_command(workflow_command, current_dir)?;
    let context = resolve_project_context(source_argument.as_deref(), current_dir)?;
    context.apply_runtime_environment()?;

    let _working_directory = WorkingDirectoryGuard::change_to(&context.project_root)?;
    let program = load_checked_program(&context.source_path)?;
    let program = optimize_checked_program(&program, &context.source_path)?;

    let workflows = collect_workflows(&program);
    let output = match workflow_behavior {
        ResolvedWorkflowCommand::List => render_workflow_list(&workflows),
        ResolvedWorkflowCommand::Run { workflow_name } => {
            run_workflow(&program, &workflows, workflow_name.as_deref())?
        }
    };

    if !output.is_empty() {
        println!("{output}");
    }

    Ok(())
}

fn run_doctor(current_dir: &Path) -> Result<(), CliError> {
    let report = collect_doctor_report(current_dir)?;
    let rendered = render_doctor_report(&report);
    if report.has_critical_errors() {
        Err(CliError::new(rendered))
    } else {
        println!("{rendered}");
        Ok(())
    }
}

fn run_deps(current_dir: &Path) -> Result<(), CliError> {
    let project_root = find_project_root_from_dir(current_dir).ok_or_else(|| {
        CliError::new("no loz.toml found in the current directory or any parent directory")
    })?;
    let package = PackageInfo::load_root_package(&project_root)?;
    let dependencies = package.resolve_direct_dependencies()?;

    println!("{}", render_dependencies(&dependencies));
    Ok(())
}

fn render_dependencies(dependencies: &[ResolvedDependency]) -> String {
    if dependencies.is_empty() {
        return "No dependencies found.".to_string();
    }

    let mut lines = vec!["Dependencies:".to_string(), String::new()];
    for dependency in dependencies {
        lines.push(dependency.alias.clone());
        lines.push(format!("  path: {}", dependency.path_text));
        lines.push(format!("  main: {}", dependency.main_relative.display()));
        lines.push(String::new());
    }

    lines.join("\n").trim_end().to_string()
}

fn run_init(current_dir: &Path, project_path: &Path) -> Result<(), CliError> {
    let target_dir = resolve_path_from_cwd(current_dir, project_path);
    let project_name = infer_project_name(&target_dir)?;

    if target_dir.exists() {
        if !target_dir.is_dir() {
            return Err(CliError::new(format!(
                "cannot initialize project at '{}': path exists and is not a directory",
                target_dir.display()
            )));
        }

        let mut entries = fs::read_dir(&target_dir).map_err(|error| {
            CliError::new(format!(
                "failed to inspect project directory '{}': {error}",
                target_dir.display()
            ))
        })?;
        if entries
            .next()
            .transpose()
            .map_err(|error| {
                CliError::new(format!(
                    "failed to inspect project directory '{}': {error}",
                    target_dir.display()
                ))
            })?
            .is_some()
        {
            return Err(CliError::new(format!(
                "cannot initialize project at '{}': directory already exists and is not empty",
                target_dir.display()
            )));
        }
    }

    fs::create_dir_all(target_dir.join("src")).map_err(|error| {
        CliError::new(format!(
            "failed to create project source directory '{}': {error}",
            target_dir.join("src").display()
        ))
    })?;
    fs::create_dir_all(target_dir.join("tools")).map_err(|error| {
        CliError::new(format!(
            "failed to create tools directory '{}': {error}",
            target_dir.join("tools").display()
        ))
    })?;
    fs::create_dir_all(target_dir.join("examples")).map_err(|error| {
        CliError::new(format!(
            "failed to create examples directory '{}': {error}",
            target_dir.join("examples").display()
        ))
    })?;
    fs::create_dir_all(target_dir.join("packages")).map_err(|error| {
        CliError::new(format!(
            "failed to create packages directory '{}': {error}",
            target_dir.join("packages").display()
        ))
    })?;

    fs::write(target_dir.join("loz.toml"), init_loz_toml(&project_name)).map_err(|error| {
        CliError::new(format!(
            "failed to write project config '{}': {error}",
            target_dir.join("loz.toml").display()
        ))
    })?;
    fs::write(target_dir.join(".env.example"), init_dotenv_example()).map_err(|error| {
        CliError::new(format!(
            "failed to write dotenv template '{}': {error}",
            target_dir.join(".env.example").display()
        ))
    })?;
    fs::write(target_dir.join("src/main.loz"), init_main_source()).map_err(|error| {
        CliError::new(format!(
            "failed to write source file '{}': {error}",
            target_dir.join("src/main.loz").display()
        ))
    })?;
    fs::write(target_dir.join("tools/tools.py"), init_python_tools()).map_err(|error| {
        CliError::new(format!(
            "failed to write Python tools file '{}': {error}",
            target_dir.join("tools/tools.py").display()
        ))
    })?;
    fs::write(target_dir.join("examples/hello.loz"), init_hello_example()).map_err(|error| {
        CliError::new(format!(
            "failed to write example file '{}': {error}",
            target_dir.join("examples/hello.loz").display()
        ))
    })?;
    fs::write(target_dir.join("README.md"), init_readme(&project_name)).map_err(|error| {
        CliError::new(format!(
            "failed to write README '{}': {error}",
            target_dir.join("README.md").display()
        ))
    })?;
    fs::write(
        target_dir.join("packages/README.md"),
        init_packages_readme(),
    )
    .map_err(|error| {
        CliError::new(format!(
            "failed to write packages README '{}': {error}",
            target_dir.join("packages/README.md").display()
        ))
    })?;

    println!("Initialized Loz project at '{}'", target_dir.display());
    Ok(())
}

fn infer_project_name(target_dir: &Path) -> Result<String, CliError> {
    let name = target_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            CliError::new(format!(
                "cannot infer project name from path '{}'",
                target_dir.display()
            ))
        })?;
    Ok(name.to_string())
}

fn init_loz_toml(project_name: &str) -> String {
    format!(
        r#"[project]
name = "{project_name}"
version = "0.1.0"
main = "src/main.loz"

# [dependencies]
# text_utils = {{ path = "./packages/text_utils" }}

[llm]
provider = "mock"
model = "qwen2.5:0.5b"

[ollama]
base_url = "http://localhost:11434"

[github]
token_env = "GITHUB_TOKEN"
models_base_url = "https://models.github.ai/inference"

[python]
path = "python3"
"#
    )
}

fn init_dotenv_example() -> &'static str {
    r#"LOZ_LLM_PROVIDER=mock
LOZ_MODEL=qwen2.5:0.5b
LOZ_OLLAMA_BASE_URL=http://localhost:11434
LOZ_PYTHON_PATH=python3
GITHUB_TOKEN=
"#
}

fn init_packages_readme() -> &'static str {
    "Local Loz packages can be placed in this directory and referenced from [dependencies] in loz.toml.\n"
}

fn init_main_source() -> &'static str {
    r#"tool demo_user() -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

agent SupportAgent {
    model: "mock";

    tools: [
        demo_user
    ];

    task answer(question: Text) -> Text {
        const user: Json = demo_user();
        return llm.ask(json.stringify(user));
    }
}

workflow DemoFlow {
    step demo_user;
}

func main() -> i32 {
    print("Loz project ready");
    return 0;
}
"#
}

fn init_python_tools() -> &'static str {
    r#"def analyze_text(payload):
    text = payload["text"]
    return {
        "length": len(text),
        "label": "ok",
    }
"#
}

fn init_hello_example() -> &'static str {
    r#"func main() -> i32 {
    print("Hello from Loz");
    return 0;
}
"#
}

fn init_readme(project_name: &str) -> String {
    format!(
        r#"# {project_name}

## Commands

```bash
loz run
loz check
loz agent list
LOZ_LLM_PROVIDER=mock loz agent run "hello"
loz workflow list
loz workflow run
```
"#
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlatformOs {
    Linux,
    Macos,
    Windows,
}

impl PlatformOs {
    fn label(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BuildPlatform {
    os: PlatformOs,
    exe_suffix: &'static str,
    object_suffix: &'static str,
    static_lib_suffix: &'static str,
    dynamic_lib_suffix: &'static str,
    default_clang: &'static str,
    default_llc: &'static str,
}

impl BuildPlatform {
    fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::windows()
        } else if cfg!(target_os = "macos") {
            Self::macos()
        } else {
            Self::linux()
        }
    }

    fn linux() -> Self {
        Self {
            os: PlatformOs::Linux,
            exe_suffix: "",
            object_suffix: ".o",
            static_lib_suffix: ".a",
            dynamic_lib_suffix: ".so",
            default_clang: "clang",
            default_llc: "llc",
        }
    }

    fn macos() -> Self {
        Self {
            os: PlatformOs::Macos,
            exe_suffix: "",
            object_suffix: ".o",
            static_lib_suffix: ".a",
            dynamic_lib_suffix: ".dylib",
            default_clang: "clang",
            default_llc: "llc",
        }
    }

    fn windows() -> Self {
        Self {
            os: PlatformOs::Windows,
            exe_suffix: ".exe",
            object_suffix: ".obj",
            static_lib_suffix: ".lib",
            dynamic_lib_suffix: ".dll",
            default_clang: "clang",
            default_llc: "llc",
        }
    }

    fn executable_name(self, stem: &str) -> String {
        format!("{stem}{}", self.exe_suffix)
    }

    fn staged_executable_name(self, stem: &str, build_token: &str) -> String {
        format!(".{stem}.{build_token}{}", self.exe_suffix)
    }

    fn object_file_name(self, stem: &str) -> String {
        format!("{stem}{}", self.object_suffix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTool {
    name: String,
    path: PathBuf,
    source: ToolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolSource {
    EnvOverride(&'static str),
    Path,
}

impl ToolSource {
    fn label(self) -> &'static str {
        match self {
            Self::EnvOverride(env_key) => env_key,
            Self::Path => "PATH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    NativeBuildOnly,
    RuntimeOptional,
}

impl ToolKind {
    fn missing_severity(self) -> DoctorSeverity {
        match self {
            Self::NativeBuildOnly | Self::RuntimeOptional => DoctorSeverity::Warning,
        }
    }

    fn failed_severity(self) -> DoctorSeverity {
        self.missing_severity()
    }
}

#[derive(Debug)]
struct TempBuildDir {
    path: PathBuf,
}

impl TempBuildDir {
    fn create(prefix: &str, build_token: &str) -> Result<Self, CliError> {
        let path = env::temp_dir().join(format!("{prefix}_{build_token}"));
        fs::create_dir_all(&path).map_err(|error| {
            CliError::new(format!(
                "failed to create temporary build directory '{}': {error}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempBuildDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorSeverity {
    Ok,
    Warning,
    Error,
}

impl DoctorSeverity {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorItem {
    label: String,
    severity: DoctorSeverity,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorSection {
    title: String,
    items: Vec<DoctorItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorReport {
    sections: Vec<DoctorSection>,
}

impl DoctorReport {
    fn has_critical_errors(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|section| section.items.iter())
            .any(|item| item.severity == DoctorSeverity::Error)
    }
}

fn collect_doctor_report(current_dir: &Path) -> Result<DoctorReport, CliError> {
    let platform = BuildPlatform::current();
    let project_root = find_project_root_from_dir(current_dir)
        .map(|path| canonicalize_existing_dir(&path))
        .transpose()?
        .unwrap_or_else(|| current_dir.to_path_buf());
    let config = load_project_file_config(&project_root)?;
    let dotenv_values = load_dotenv_values(&project_root)?;

    let platform_section = collect_doctor_platform_section(platform);
    let project_section = collect_doctor_project_section(&project_root, &config);
    let toolchain_section = collect_doctor_toolchain_section(platform);
    let runtime_section =
        collect_doctor_runtime_section(platform, &project_root, &config, &dotenv_values);
    let status_section = collect_doctor_status_section(
        [
            &platform_section,
            &project_section,
            &toolchain_section,
            &runtime_section,
        ],
        native_build_ready(&toolchain_section),
    );

    Ok(DoctorReport {
        sections: vec![
            platform_section,
            project_section,
            toolchain_section,
            runtime_section,
            status_section,
        ],
    })
}

fn collect_doctor_platform_section(platform: BuildPlatform) -> DoctorSection {
    DoctorSection {
        title: "Platform".to_string(),
        items: vec![
            DoctorItem {
                label: "os".to_string(),
                severity: DoctorSeverity::Ok,
                message: platform.os.label().to_string(),
            },
            DoctorItem {
                label: "executable suffix".to_string(),
                severity: DoctorSeverity::Ok,
                message: if platform.exe_suffix.is_empty() {
                    "none".to_string()
                } else {
                    platform.exe_suffix.to_string()
                },
            },
            DoctorItem {
                label: "object suffix".to_string(),
                severity: DoctorSeverity::Ok,
                message: platform.object_suffix.to_string(),
            },
        ],
    }
}

fn collect_doctor_project_section(
    project_root: &Path,
    config: &ProjectFileConfig,
) -> DoctorSection {
    let loz_toml_path = project_root.join("loz.toml");
    let dotenv_path = project_root.join(".env");

    let mut items = vec![
        DoctorItem {
            label: "root".to_string(),
            severity: DoctorSeverity::Ok,
            message: project_root.display().to_string(),
        },
        DoctorItem {
            label: "loz.toml".to_string(),
            severity: if loz_toml_path.is_file() {
                DoctorSeverity::Ok
            } else {
                DoctorSeverity::Warning
            },
            message: if loz_toml_path.is_file() {
                "found".to_string()
            } else {
                "missing".to_string()
            },
        },
        DoctorItem {
            label: ".env".to_string(),
            severity: if dotenv_path.is_file() {
                DoctorSeverity::Ok
            } else {
                DoctorSeverity::Warning
            },
            message: if dotenv_path.is_file() {
                "found".to_string()
            } else {
                "missing".to_string()
            },
        },
    ];

    if loz_toml_path.is_file() {
        match config
            .project
            .as_ref()
            .and_then(|project| project.main.as_deref())
        {
            Some(main) => {
                let main_path = resolve_project_relative_path(project_root, main);
                items.push(DoctorItem {
                    label: "main".to_string(),
                    severity: if main_path.is_file() {
                        DoctorSeverity::Ok
                    } else {
                        DoctorSeverity::Error
                    },
                    message: if main_path.is_file() {
                        main.to_string()
                    } else {
                        format!("{main} (missing)")
                    },
                });
            }
            None => items.push(DoctorItem {
                label: "main".to_string(),
                severity: DoctorSeverity::Error,
                message: "missing [project].main".to_string(),
            }),
        }
    } else {
        items.push(DoctorItem {
            label: "main".to_string(),
            severity: DoctorSeverity::Warning,
            message: "not configured".to_string(),
        });
    }

    DoctorSection {
        title: "Project".to_string(),
        items,
    }
}

fn collect_doctor_toolchain_section(platform: BuildPlatform) -> DoctorSection {
    DoctorSection {
        title: "Toolchain".to_string(),
        items: vec![
            check_tool(
                "cargo",
                Some("LOZ_CARGO_PATH"),
                &["cargo"],
                &["--version"],
                ToolKind::NativeBuildOnly,
            ),
            check_tool(
                "clang",
                Some("LOZ_CLANG_PATH"),
                &[platform.default_clang],
                &["--version"],
                ToolKind::NativeBuildOnly,
            ),
            check_tool(
                "llc",
                Some("LOZ_LLC_PATH"),
                &[platform.default_llc],
                &["--version"],
                ToolKind::NativeBuildOnly,
            ),
        ],
    }
}

fn collect_doctor_runtime_section(
    _platform: BuildPlatform,
    project_root: &Path,
    config: &ProjectFileConfig,
    dotenv_values: &HashMap<String, String>,
) -> DoctorSection {
    let provider = config_value_with_precedence(
        "LOZ_LLM_PROVIDER",
        dotenv_values,
        config.llm.as_ref().and_then(|llm| llm.provider.as_deref()),
        Some("mock"),
    );
    let configured_python = env::var("LOZ_PYTHON_PATH")
        .ok()
        .or_else(|| dotenv_values.get("LOZ_PYTHON_PATH").cloned())
        .or_else(|| {
            config
                .python
                .as_ref()
                .and_then(|python| python.path.clone())
        });
    let ollama_base_url = config_value_with_precedence(
        "LOZ_OLLAMA_BASE_URL",
        dotenv_values,
        config
            .ollama
            .as_ref()
            .and_then(|ollama| ollama.base_url.as_deref()),
        Some("http://localhost:11434"),
    );
    let github_token_env = config_value_with_precedence(
        "LOZ_GITHUB_TOKEN_ENV",
        dotenv_values,
        config
            .github
            .as_ref()
            .and_then(|github| github.token_env.as_deref()),
        Some("GITHUB_TOKEN"),
    );

    let mut items = vec![
        check_python_tool(configured_python.as_deref()),
        DoctorItem {
            label: "provider".to_string(),
            severity: DoctorSeverity::Ok,
            message: provider.clone(),
        },
    ];

    let should_check_ollama = provider == "ollama"
        || env::var_os("LOZ_OLLAMA_BASE_URL").is_some()
        || dotenv_values.contains_key("LOZ_OLLAMA_BASE_URL")
        || config
            .ollama
            .as_ref()
            .and_then(|ollama| ollama.base_url.as_ref())
            .is_some();
    if should_check_ollama {
        items.push(check_ollama(&ollama_base_url));
    } else {
        items.push(DoctorItem {
            label: "ollama".to_string(),
            severity: DoctorSeverity::Warning,
            message: "not configured".to_string(),
        });
    }

    items.push(check_github_token(
        &provider,
        project_root,
        dotenv_values,
        &github_token_env,
    ));

    DoctorSection {
        title: "Runtime".to_string(),
        items,
    }
}

fn collect_doctor_status_section<'a>(
    sections: impl IntoIterator<Item = &'a DoctorSection>,
    native_build_ready: bool,
) -> DoctorSection {
    let mut has_errors = false;
    let mut has_warnings = false;

    for section in sections {
        for item in &section.items {
            match item.severity {
                DoctorSeverity::Ok => {}
                DoctorSeverity::Warning => has_warnings = true,
                DoctorSeverity::Error => has_errors = true,
            }
        }
    }

    let (severity, message) = if has_errors {
        (DoctorSeverity::Error, "not ready")
    } else if !native_build_ready {
        (
            DoctorSeverity::Warning,
            "ready for interpreter, native build unavailable",
        )
    } else if has_warnings {
        (DoctorSeverity::Warning, "ready with warnings")
    } else {
        (DoctorSeverity::Ok, "ready")
    };

    DoctorSection {
        title: "Status".to_string(),
        items: vec![DoctorItem {
            label: "result".to_string(),
            severity,
            message: message.to_string(),
        }],
    }
}

fn render_doctor_report(report: &DoctorReport) -> String {
    let mut lines = vec!["Loz doctor".to_string()];

    for section in &report.sections {
        lines.push(String::new());
        lines.push(format!("{}:", section.title));
        for item in &section.items {
            lines.push(format!(
                "  {}: {} - {}",
                item.label,
                item.severity.label(),
                item.message
            ));
        }
    }

    lines.join("\n")
}

fn native_build_ready(toolchain_section: &DoctorSection) -> bool {
    ["cargo", "clang", "llc"].iter().all(|label| {
        toolchain_section
            .items
            .iter()
            .find(|item| item.label == *label)
            .is_some_and(|item| item.severity == DoctorSeverity::Ok)
    })
}

fn check_tool(
    label: &str,
    env_override: Option<&'static str>,
    candidates: &[&str],
    args: &[&str],
    kind: ToolKind,
) -> DoctorItem {
    let resolved = match find_tool(env_override, candidates, label) {
        Ok(resolved) => resolved,
        Err(message) => {
            return DoctorItem {
                label: label.to_string(),
                severity: kind.missing_severity(),
                message,
            };
        }
    };

    let result = Command::new(&resolved.path).args(args).output();
    match result {
        Ok(output) if output.status.success() => DoctorItem {
            label: label.to_string(),
            severity: DoctorSeverity::Ok,
            message: format!("ok ({})", resolved.path.display()),
        },
        Ok(_) => DoctorItem {
            label: label.to_string(),
            severity: kind.failed_severity(),
            message: format!(
                "found at '{}' via {}, but returned a non-zero status",
                resolved.path.display(),
                resolved.source.label()
            ),
        },
        Err(error) => DoctorItem {
            label: label.to_string(),
            severity: kind.failed_severity(),
            message: format!(
                "found at '{}' via {}, but could not be executed ({error})",
                resolved.path.display(),
                resolved.source.label()
            ),
        },
    }
}

fn check_python_tool(configured_python: Option<&str>) -> DoctorItem {
    let resolved = match find_python_tool(configured_python) {
        Ok(resolved) => resolved,
        Err(message) => {
            return DoctorItem {
                label: "python".to_string(),
                severity: ToolKind::RuntimeOptional.missing_severity(),
                message,
            };
        }
    };

    let result = Command::new(&resolved.path).arg("--version").output();
    match result {
        Ok(output) if output.status.success() => DoctorItem {
            label: "python".to_string(),
            severity: DoctorSeverity::Ok,
            message: format!("ok ({})", resolved.path.display()),
        },
        Ok(_) => DoctorItem {
            label: "python".to_string(),
            severity: ToolKind::RuntimeOptional.failed_severity(),
            message: format!(
                "found at '{}' via {}, but returned a non-zero status",
                resolved.path.display(),
                resolved.source.label()
            ),
        },
        Err(error) => DoctorItem {
            label: "python".to_string(),
            severity: ToolKind::RuntimeOptional.failed_severity(),
            message: format!(
                "found at '{}' via {}, but could not be executed ({error})",
                resolved.path.display(),
                resolved.source.label()
            ),
        },
    }
}

fn check_ollama(base_url: &str) -> DoctorItem {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = match Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(client) => client,
        Err(error) => {
            return DoctorItem {
                label: "ollama".to_string(),
                severity: DoctorSeverity::Warning,
                message: format!("failed to initialize HTTP client ({error})"),
            };
        }
    };

    match client.get(&url).send() {
        Ok(response) if response.status().is_success() => DoctorItem {
            label: "ollama".to_string(),
            severity: DoctorSeverity::Ok,
            message: format!("reachable at {base_url}"),
        },
        Ok(response) => DoctorItem {
            label: "ollama".to_string(),
            severity: DoctorSeverity::Warning,
            message: format!("not reachable at {base_url} (status {})", response.status()),
        },
        Err(_) => DoctorItem {
            label: "ollama".to_string(),
            severity: DoctorSeverity::Warning,
            message: format!("not reachable at {base_url}"),
        },
    }
}

fn check_github_token(
    provider: &str,
    _project_root: &Path,
    dotenv_values: &HashMap<String, String>,
    token_env_name: &str,
) -> DoctorItem {
    let token_present = env::var(token_env_name)
        .ok()
        .map(|value| !value.is_empty())
        .unwrap_or(false)
        || dotenv_values
            .get(token_env_name)
            .map(|value| !value.is_empty())
            .unwrap_or(false);
    let severity = if token_present {
        DoctorSeverity::Ok
    } else if provider == "github" {
        DoctorSeverity::Warning
    } else {
        DoctorSeverity::Warning
    };
    let message = if token_present {
        format!("found in {token_env_name}")
    } else {
        format!("missing ({token_env_name})")
    };

    DoctorItem {
        label: "github token".to_string(),
        severity,
        message,
    }
}

fn config_value_with_precedence(
    env_key: &str,
    dotenv_values: &HashMap<String, String>,
    config_value: Option<&str>,
    default_value: Option<&str>,
) -> String {
    env::var(env_key)
        .ok()
        .or_else(|| dotenv_values.get(env_key).cloned())
        .or_else(|| config_value.map(str::to_string))
        .or_else(|| default_value.map(str::to_string))
        .unwrap_or_default()
}

fn resolve_workflow_command(
    workflow_command: WorkflowCommandInvocation,
    current_dir: &Path,
) -> Result<(Option<PathBuf>, ResolvedWorkflowCommand), CliError> {
    match workflow_command {
        WorkflowCommandInvocation::List { source_argument } => {
            Ok((source_argument, ResolvedWorkflowCommand::List))
        }
        WorkflowCommandInvocation::Run { raw_arguments } => {
            let (source_argument, consumed_arguments) =
                split_workflow_run_source_argument(&raw_arguments, current_dir);
            let remaining_arguments = &raw_arguments[consumed_arguments..];
            if remaining_arguments.len() > 1 {
                return Err(CliError::new(
                    "usage: loz workflow run [source.loz] [WorkflowName]",
                ));
            }

            Ok((
                source_argument,
                ResolvedWorkflowCommand::Run {
                    workflow_name: remaining_arguments.first().cloned(),
                },
            ))
        }
    }
}

fn split_workflow_run_source_argument(
    raw_arguments: &[String],
    current_dir: &Path,
) -> (Option<PathBuf>, usize) {
    split_source_argument(raw_arguments, current_dir)
}

fn split_agent_run_source_argument(
    raw_arguments: &[String],
    current_dir: &Path,
) -> (Option<PathBuf>, usize) {
    split_source_argument(raw_arguments, current_dir)
}

fn split_source_argument(raw_arguments: &[String], current_dir: &Path) -> (Option<PathBuf>, usize) {
    let Some(first_argument) = raw_arguments.first() else {
        return (None, 0);
    };

    let candidate = PathBuf::from(first_argument);
    let resolved_path = resolve_path_from_cwd(current_dir, &candidate);
    let is_loz_file = resolved_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("loz"))
        .unwrap_or(false);

    if is_loz_file && resolved_path.is_file() {
        (Some(candidate), 1)
    } else {
        (None, 0)
    }
}

fn collect_agents(program: &Program) -> Vec<DiscoveredAgent> {
    program
        .statements
        .iter()
        .filter_map(|statement| {
            let Statement::AgentDeclaration(AgentDeclaration {
                name,
                model,
                tools,
                tasks,
                ..
            }) = statement
            else {
                return None;
            };

            Some(DiscoveredAgent {
                name: name.clone(),
                model: model.as_ref().map(format_metadata_expression),
                tools: extract_agent_tools(tools.as_ref()),
                tasks: tasks
                    .iter()
                    .map(|task| DiscoveredTask {
                        name: task.name.clone(),
                        parameters: task.parameters.clone(),
                        return_type: task.return_type.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

fn collect_workflows(program: &Program) -> Vec<DiscoveredWorkflow> {
    program
        .statements
        .iter()
        .filter_map(|statement| {
            let Statement::WorkflowDeclaration(WorkflowDeclaration { name, steps, .. }) = statement
            else {
                return None;
            };

            Some(DiscoveredWorkflow {
                name: name.clone(),
                steps: steps
                    .iter()
                    .map(|step| DiscoveredWorkflowStep {
                        name: step.name.clone(),
                        target: step.target.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

fn render_agent_list(agents: &[DiscoveredAgent]) -> String {
    if agents.is_empty() {
        return "No agents found.".to_string();
    }

    let mut output = String::from("Agents found:\n");
    for agent in agents {
        output.push('\n');
        output.push_str(&agent.name);
        output.push('\n');
        output.push_str("  model: ");
        output.push_str(agent.model.as_deref().unwrap_or("(none)"));
        output.push('\n');
        output.push_str("  tools:\n");
        if agent.tools.is_empty() {
            output.push_str("    (none)\n");
        } else {
            for tool in &agent.tools {
                output.push_str("    ");
                output.push_str(tool);
                output.push('\n');
            }
        }
        output.push_str("  tasks:\n");
        if agent.tasks.is_empty() {
            output.push_str("    (none)\n");
        } else {
            for task in &agent.tasks {
                output.push_str("    ");
                output.push_str(&format_task_signature(task));
                output.push('\n');
            }
        }
    }

    output.trim_end().to_string()
}

fn render_workflow_list(workflows: &[DiscoveredWorkflow]) -> String {
    if workflows.is_empty() {
        return "No workflows found.".to_string();
    }

    let mut output = String::from("Workflows found:\n");
    for workflow in workflows {
        output.push('\n');
        output.push_str(&workflow.name);
        output.push('\n');
        output.push_str("  steps:\n");
        for (index, step) in workflow.steps.iter().enumerate() {
            output.push_str("    ");
            output.push_str(&(index + 1).to_string());
            output.push_str(". ");
            output.push_str(&step.name);
            output.push('\n');
        }
    }

    output.trim_end().to_string()
}

fn run_agent_task(
    program: &Program,
    agents: &[DiscoveredAgent],
    cli_arguments: &[String],
) -> Result<String, CliError> {
    let selection = resolve_agent_task_selection(agents, cli_arguments)?;
    let parsed_arguments = parse_task_arguments(selection.task, selection.arguments)?;
    let mut interpreter = Interpreter::new();
    let result = interpreter
        .execute_agent_task(
            program,
            &selection.agent.name,
            &selection.task.name,
            parsed_arguments,
        )
        .map_err(|error| CliError::new(error.to_string()))?;

    format_task_result(selection.task, result)
}

fn run_workflow(
    program: &Program,
    workflows: &[DiscoveredWorkflow],
    workflow_name: Option<&str>,
) -> Result<String, CliError> {
    let workflow = resolve_workflow_selection(workflows, workflow_name)?;
    let mut interpreter = Interpreter::new();
    let outcomes = interpreter
        .execute_workflow(program, &workflow.name)
        .map_err(|error| CliError::new(error.to_string()))?;

    Ok(render_workflow_run_output(&workflow.name, &outcomes))
}

fn resolve_agent_task_selection<'a>(
    agents: &'a [DiscoveredAgent],
    cli_arguments: &'a [String],
) -> Result<AgentTaskSelection<'a>, CliError> {
    if cli_arguments.len() >= 2 {
        let agent_name = &cli_arguments[0];
        let task_name = &cli_arguments[1];
        let agent = agents
            .iter()
            .find(|agent| agent.name == *agent_name)
            .ok_or_else(|| unknown_agent_error(agent_name, agents))?;
        let task = agent
            .tasks
            .iter()
            .find(|task| task.name == *task_name)
            .ok_or_else(|| unknown_task_error(agent, task_name))?;

        return Ok(AgentTaskSelection {
            agent,
            task,
            arguments: &cli_arguments[2..],
        });
    }

    if agents.is_empty() {
        return Err(CliError::new("error: no agents found"));
    }

    if agents.len() != 1 || agents[0].tasks.len() != 1 {
        return Err(ambiguous_agent_selection_error(agents));
    }

    Ok(AgentTaskSelection {
        agent: &agents[0],
        task: &agents[0].tasks[0],
        arguments: cli_arguments,
    })
}

fn resolve_workflow_selection<'a>(
    workflows: &'a [DiscoveredWorkflow],
    workflow_name: Option<&str>,
) -> Result<&'a DiscoveredWorkflow, CliError> {
    if let Some(workflow_name) = workflow_name {
        return workflows
            .iter()
            .find(|workflow| workflow.name == workflow_name)
            .ok_or_else(|| {
                let mut message = format!("error: workflow '{}' not found", workflow_name);
                message.push_str("\n\n");
                message.push_str(&format_available_workflows(workflows));
                CliError::new(message)
            });
    }

    match workflows {
        [] => Err(CliError::new("No workflows found.")),
        [workflow] => Ok(workflow),
        _ => {
            let mut message =
                String::from("error: cannot auto-select workflow because multiple workflows exist");
            message.push_str("\n\n");
            message.push_str(&format_available_workflows(workflows));
            message.push_str("\n\nUse:\n  loz workflow run <WorkflowName>");
            Err(CliError::new(message))
        }
    }
}

fn parse_task_arguments(
    task: &DiscoveredTask,
    cli_arguments: &[String],
) -> Result<Vec<RuntimeValue>, CliError> {
    if task.parameters.len() != cli_arguments.len() {
        return Err(CliError::new(format!(
            "error: task '{}' expects {} argument{}, got {}\n\nUsage:\n  {}",
            task.name,
            task.parameters.len(),
            if task.parameters.len() == 1 { "" } else { "s" },
            cli_arguments.len(),
            task_usage_line(task)
        )));
    }

    task.parameters
        .iter()
        .zip(cli_arguments.iter())
        .enumerate()
        .map(|(index, (parameter, raw_value))| {
            parse_task_argument(parameter, raw_value).map_err(|_| {
                CliError::new(format!(
                    "error: argument {} for task '{}' must be {}, got '{}'\n\nUsage:\n  {}",
                    index + 1,
                    task.name,
                    type_name_label(&parameter.type_name),
                    raw_value,
                    task_usage_line(task)
                ))
            })
        })
        .collect()
}

fn parse_task_argument(
    parameter: &FunctionParameter,
    raw_value: &str,
) -> Result<RuntimeValue, CliError> {
    match &parameter.type_name {
        TypeName::Text => Ok(RuntimeValue::Text(raw_value.to_string())),
        TypeName::I32 => raw_value
            .parse::<i32>()
            .map(|value| RuntimeValue::Int(i64::from(value)))
            .map_err(|_| CliError::new("invalid i32 argument")),
        TypeName::I64 => raw_value
            .parse::<i64>()
            .map(RuntimeValue::Int)
            .map_err(|_| CliError::new("invalid i64 argument")),
        TypeName::F64 => raw_value
            .parse::<f64>()
            .map(RuntimeValue::Float)
            .map_err(|_| CliError::new("invalid f64 argument")),
        TypeName::Bool => parse_bool_argument(raw_value)
            .map(RuntimeValue::Bool)
            .ok_or_else(|| CliError::new("invalid bool argument")),
        TypeName::Json => serde_json::from_str::<JsonValue>(raw_value)
            .map(RuntimeValue::Json)
            .map_err(|_| CliError::new("invalid json argument")),
        _ => Err(CliError::new(format!(
            "unsupported CLI parameter type '{}'",
            type_name_label(&parameter.type_name)
        ))),
    }
}

fn parse_bool_argument(raw_value: &str) -> Option<bool> {
    match raw_value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn format_task_result(task: &DiscoveredTask, value: RuntimeValue) -> Result<String, CliError> {
    format_runtime_value(&value).ok_or_else(|| {
        CliError::new(format!(
            "error: task '{}' returned unsupported type '{}'",
            task.name,
            type_name_label(&task.return_type)
        ))
    })
}

fn format_runtime_value(value: &RuntimeValue) -> Option<String> {
    match value {
        RuntimeValue::Text(value) => Some(value.clone()),
        RuntimeValue::Int(value) => Some(value.to_string()),
        RuntimeValue::Float(value) => Some(RuntimeValue::Float(*value).to_string()),
        RuntimeValue::Bool(value) => Some(value.to_string()),
        RuntimeValue::Json(value) => Some(value.to_string()),
        RuntimeValue::Void => Some(String::new()),
        _ => None,
    }
}

fn format_task_signature(task: &DiscoveredTask) -> String {
    format!(
        "{}({}) -> {}",
        task.name,
        task.parameters
            .iter()
            .map(|parameter| format!(
                "{}: {}",
                parameter.name,
                type_name_label(&parameter.type_name)
            ))
            .collect::<Vec<_>>()
            .join(", "),
        type_name_label(&task.return_type)
    )
}

fn task_usage_line(task: &DiscoveredTask) -> String {
    let parameter_usage = task
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "<{}: {}>",
                parameter.name,
                type_name_label(&parameter.type_name)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");

    if parameter_usage.is_empty() {
        format!("loz agent run <AgentName> <TaskName>")
    } else {
        format!("loz agent run <AgentName> <TaskName> {parameter_usage}")
    }
}

fn unknown_agent_error(agent_name: &str, agents: &[DiscoveredAgent]) -> CliError {
    let mut message = format!("error: agent '{}' not found", agent_name);
    message.push_str("\n\n");
    message.push_str(&format_available_agents(agents));
    CliError::new(message)
}

fn unknown_task_error(agent: &DiscoveredAgent, task_name: &str) -> CliError {
    let mut message = format!(
        "error: task '{}' not found in agent '{}'",
        task_name, agent.name
    );
    message.push_str("\n\nAvailable tasks:\n");
    for task in &agent.tasks {
        message.push_str("  ");
        message.push_str(&format_task_signature(task));
        message.push('\n');
    }
    CliError::new(message.trim_end().to_string())
}

fn ambiguous_agent_selection_error(agents: &[DiscoveredAgent]) -> CliError {
    let mut message =
        String::from("error: cannot auto-select agent/task because multiple candidates exist");

    if agents.len() == 1 {
        message.push_str(&format!(
            "\n\nAvailable tasks in agent '{}':\n",
            agents[0].name
        ));
        for task in &agents[0].tasks {
            message.push_str("  ");
            message.push_str(&format_task_signature(task));
            message.push('\n');
        }
    } else {
        message.push_str("\n\n");
        message.push_str(&format_available_agents(agents));
        message.push('\n');
    }

    message.push_str("\nUse:\n  loz agent run <AgentName> <TaskName> <args...>");
    CliError::new(message)
}

fn format_available_agents(agents: &[DiscoveredAgent]) -> String {
    if agents.is_empty() {
        return "No agents found.".to_string();
    }

    let mut message = String::from("Available agents:\n");
    for agent in agents {
        message.push_str("  ");
        message.push_str(&agent.name);
        message.push('\n');
    }
    message.trim_end().to_string()
}

fn format_available_workflows(workflows: &[DiscoveredWorkflow]) -> String {
    if workflows.is_empty() {
        return "No workflows found.".to_string();
    }

    let mut message = String::from("Available workflows:\n");
    for workflow in workflows {
        message.push_str("  ");
        message.push_str(&workflow.name);
        message.push('\n');
    }
    message.trim_end().to_string()
}

fn render_workflow_run_output(_workflow_name: &str, outcomes: &[WorkflowStepOutcome]) -> String {
    let total_steps = outcomes.len();
    let mut lines = Vec::new();

    for (index, outcome) in outcomes.iter().enumerate() {
        lines.push(format!(
            "[{}/{}] {}",
            index + 1,
            total_steps,
            outcome.step_name
        ));
        if let Some(result) = &outcome.result {
            if let Some(text) = format_runtime_value(result) {
                if !text.is_empty() {
                    lines.push(text);
                }
            }
        }
    }

    lines.join("\n")
}

fn extract_agent_tools(tools_expression: Option<&Expression>) -> Vec<String> {
    let Some(tools_expression) = tools_expression else {
        return Vec::new();
    };

    match &tools_expression.kind {
        ExpressionKind::ArrayLiteral(array) => array
            .elements
            .iter()
            .map(format_metadata_expression)
            .collect(),
        _ => vec![format_metadata_expression(tools_expression)],
    }
}

fn format_metadata_expression(expression: &Expression) -> String {
    match &expression.kind {
        ExpressionKind::StringLiteral(value) => value.clone(),
        ExpressionKind::Identifier(value) => value.clone(),
        ExpressionKind::IntegerLiteral(value) => value.to_string(),
        ExpressionKind::FloatLiteral(value) => RuntimeValue::Float(*value).to_string(),
        ExpressionKind::BooleanLiteral(value) => value.to_string(),
        ExpressionKind::ArrayLiteral(array) => format!(
            "[{}]",
            array
                .elements
                .iter()
                .map(format_metadata_expression)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "<expression>".to_string(),
    }
}

fn type_name_label(type_name: &TypeName) -> String {
    match type_name {
        TypeName::I8 => "i8".to_string(),
        TypeName::I16 => "i16".to_string(),
        TypeName::I32 => "i32".to_string(),
        TypeName::I64 => "i64".to_string(),
        TypeName::U8 => "u8".to_string(),
        TypeName::U16 => "u16".to_string(),
        TypeName::U32 => "u32".to_string(),
        TypeName::U64 => "u64".to_string(),
        TypeName::F32 => "f32".to_string(),
        TypeName::F64 => "f64".to_string(),
        TypeName::Bool => "Bool".to_string(),
        TypeName::Text => "Text".to_string(),
        TypeName::Json => "Json".to_string(),
        TypeName::Char => "Char".to_string(),
        TypeName::Void => "Void".to_string(),
        TypeName::Reference { inner, is_mutable } => {
            if *is_mutable {
                format!("mut ref {}", type_name_label(inner))
            } else {
                format!("ref {}", type_name_label(inner))
            }
        }
        TypeName::Array(inner, Some(size)) => {
            format!("Array<{}, {}>", type_name_label(inner), size)
        }
        TypeName::Array(inner, None) => format!("Array<{}>", type_name_label(inner)),
        TypeName::Map(key, value) => {
            format!("Map<{}, {}>", type_name_label(key), type_name_label(value))
        }
        TypeName::Set(inner) => format!("Set<{}>", type_name_label(inner)),
        TypeName::Named(name) => name.clone(),
    }
}

fn load_program_from_entry(source_path: &Path) -> Result<Program, CliError> {
    let entry_path = canonicalize_existing_file(source_path)?;
    let root_package_seed = PackageInfo::load_for_entry(&entry_path)?;
    let mut loader = ModuleLoader::new(root_package_seed.clone())?;
    let root_package = loader.package(&root_package_seed.root_dir)?.clone();
    let mut statements = Vec::new();
    loader.load_file(&entry_path, &root_package, true, None, &mut statements)?;
    Ok(Program { statements })
}

struct ModuleLoader {
    loaded_paths: HashSet<PathBuf>,
    loading_stack: Vec<PathBuf>,
    modules_by_name: HashMap<String, PathBuf>,
    packages: HashMap<PathBuf, PackageInfo>,
}

impl ModuleLoader {
    fn new(root_package: PackageInfo) -> Result<Self, CliError> {
        let mut loader = Self {
            loaded_paths: HashSet::new(),
            loading_stack: Vec::new(),
            modules_by_name: HashMap::new(),
            packages: HashMap::new(),
        };
        let mut package_stack = Vec::new();
        loader.load_package_graph(root_package, &mut package_stack)?;
        Ok(loader)
    }

    fn load_file(
        &mut self,
        source_path: &Path,
        package: &PackageInfo,
        is_entry: bool,
        expected_module_name: Option<&str>,
        merged_statements: &mut Vec<Statement>,
    ) -> Result<(), CliError> {
        if self.loaded_paths.contains(source_path) {
            return Ok(());
        }

        if self.loading_stack.iter().any(|path| path == source_path) {
            return Err(CliError::new(format!(
                "circular imports are not supported in this phase: '{}'",
                source_path.display()
            )));
        }

        self.loading_stack.push(source_path.to_path_buf());

        let source = fs::read_to_string(source_path).map_err(|error| {
            CliError::new(format!(
                "failed to read source file '{}': {error}",
                source_path.display()
            ))
        })?;

        let tokens = tokenize_with_file_path(&source, source_path.display().to_string())
            .map_err(|error| CliError::from_diagnostic(error.diagnostic.clone()))?;
        let program = parse_program(tokens)
            .map_err(|error| CliError::from_diagnostic(error.diagnostic.clone()))?;

        let module_name = self.validate_module_declarations(
            source_path,
            is_entry,
            expected_module_name,
            &program,
        )?;
        let imports = self.collect_imports(source_path, &program)?;

        if let Some(module_name) = module_name {
            if let Some(existing_path) = self.modules_by_name.get(&module_name) {
                if existing_path != source_path {
                    return Err(CliError::new(format!(
                        "duplicate module '{}' declared in '{}' and '{}'",
                        module_name,
                        existing_path.display(),
                        source_path.display()
                    )));
                }
            } else {
                self.modules_by_name
                    .insert(module_name, source_path.to_path_buf());
            }
        }

        let parent_dir = source_path.parent().ok_or_else(|| {
            CliError::new(format!(
                "failed to determine parent directory for '{}'",
                source_path.display()
            ))
        })?;

        for import_declaration in imports {
            let import_name = import_declaration.module_name.as_str();
            if is_builtin_module(import_name) {
                continue;
            }

            let local_candidate = parent_dir.join(format!("{import_name}.loz"));
            let local_exists = local_candidate.is_file();
            let dependency = package.dependencies.get(import_name);

            if local_exists && dependency.is_some() {
                return Err(CliError::from_diagnostic(
                    Diagnostic::error(format!(
                        "ambiguous import '{}': both local module '{}' and dependency '{}' exist",
                        import_name,
                        local_candidate.display(),
                        import_name
                    ))
                    .with_span(import_declaration.span.clone()),
                ));
            }

            if local_exists {
                let import_path = canonicalize_existing_file(&local_candidate)?;
                self.load_file(
                    &import_path,
                    package,
                    false,
                    Some(import_name),
                    merged_statements,
                )?;
                continue;
            }

            if let Some(dependency) = dependency {
                let dependency_package = self.package(&dependency.package_root)?.clone();
                self.load_file(
                    &dependency.main_path,
                    &dependency_package,
                    false,
                    Some(import_name),
                    merged_statements,
                )?;
                continue;
            }

            return Err(CliError::from_diagnostic(
                Diagnostic::error(format!(
                    "dependency '{}' not found in loz.toml and local module '{}' does not exist",
                    import_name,
                    local_candidate.display()
                ))
                .with_span(import_declaration.span.clone()),
            ));
        }

        self.loaded_paths.insert(source_path.to_path_buf());
        merged_statements.extend(program.statements);
        self.loading_stack.pop();
        Ok(())
    }

    fn validate_module_declarations(
        &self,
        source_path: &Path,
        is_entry: bool,
        expected_module_name: Option<&str>,
        program: &Program,
    ) -> Result<Option<String>, CliError> {
        let mut module_name = None;

        for statement in &program.statements {
            if let Statement::ModuleDeclaration(declaration) = statement {
                if module_name.replace(declaration.name.clone()).is_some() {
                    return Err(CliError::new(format!(
                        "file '{}' declares more than one module",
                        source_path.display()
                    )));
                }
            }
        }

        if !is_entry {
            let declared_module = module_name.clone().ok_or_else(|| {
                CliError::new(format!(
                    "imported file '{}' must declare a module",
                    source_path.display()
                ))
            })?;
            let expected_name = expected_module_name.ok_or_else(|| {
                CliError::new(format!(
                    "failed to determine expected module name for '{}'",
                    source_path.display()
                ))
            })?;

            if declared_module != expected_name {
                return Err(CliError::new(format!(
                    "imported file '{}' declares module '{}' but expected '{}'",
                    source_path.display(),
                    declared_module,
                    expected_name
                )));
            }
        }

        Ok(module_name)
    }

    fn collect_imports(
        &self,
        source_path: &Path,
        program: &Program,
    ) -> Result<Vec<loz_ast::ImportDeclaration>, CliError> {
        let mut imports = Vec::new();
        let mut seen_imports = HashSet::new();

        for statement in &program.statements {
            if let Statement::ImportDeclaration(import_declaration) = statement {
                if !seen_imports.insert(import_declaration.module_name.clone()) {
                    return Err(CliError::new(format!(
                        "file '{}' imports module '{}' more than once",
                        source_path.display(),
                        import_declaration.module_name
                    )));
                }

                imports.push(import_declaration.clone());
            }
        }

        Ok(imports)
    }

    fn load_package_graph(
        &mut self,
        package: PackageInfo,
        package_stack: &mut Vec<String>,
    ) -> Result<(), CliError> {
        if self.packages.contains_key(&package.root_dir) {
            return Ok(());
        }

        if let Some(index) = package_stack.iter().position(|name| name == &package.name) {
            let mut cycle = package_stack[index..].to_vec();
            cycle.push(package.name.clone());
            return Err(CliError::new(format!(
                "circular dependency detected: {}",
                cycle.join(" -> ")
            )));
        }

        package_stack.push(package.name.clone());
        let dependencies = package.resolve_direct_dependencies()?;
        let mut package = package;
        package.dependencies = dependencies
            .iter()
            .map(|dependency| (dependency.alias.clone(), dependency.clone()))
            .collect();

        for dependency in &dependencies {
            let dependency_package = PackageInfo::load_dependency_package(
                &dependency.package_root,
                Some(&dependency.alias),
            )?;
            self.load_package_graph(dependency_package, package_stack)?;
        }

        package_stack.pop();
        self.packages.insert(package.root_dir.clone(), package);
        Ok(())
    }

    fn package(&self, root: &Path) -> Result<&PackageInfo, CliError> {
        self.packages.get(root).ok_or_else(|| {
            CliError::new(format!(
                "failed to resolve package metadata for '{}'",
                root.display()
            ))
        })
    }
}

#[derive(Debug, Clone)]
struct ResolvedDependency {
    alias: String,
    path_text: String,
    package_root: PathBuf,
    main_relative: PathBuf,
    main_path: PathBuf,
}

#[derive(Debug, Clone)]
struct PackageInfo {
    name: String,
    root_dir: PathBuf,
    main_relative: PathBuf,
    main_path: PathBuf,
    dependency_specs: HashMap<String, DependencySection>,
    dependencies: HashMap<String, ResolvedDependency>,
}

impl PackageInfo {
    fn load_for_entry(entry_path: &Path) -> Result<Self, CliError> {
        if let Some(project_root) = find_project_root_from_source(entry_path) {
            Self::load_root_package(&project_root)
        } else {
            let root_dir = entry_path.parent().ok_or_else(|| {
                CliError::new(format!(
                    "failed to determine parent directory for '{}'",
                    entry_path.display()
                ))
            })?;
            let name = root_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("app")
                .to_string();
            let main_relative = entry_path.file_name().map(PathBuf::from).ok_or_else(|| {
                CliError::new(format!(
                    "failed to determine source file name for '{}'",
                    entry_path.display()
                ))
            })?;

            Ok(Self {
                name,
                root_dir: root_dir.to_path_buf(),
                main_relative,
                main_path: entry_path.to_path_buf(),
                dependency_specs: HashMap::new(),
                dependencies: HashMap::new(),
            })
        }
    }

    fn load_root_package(project_root: &Path) -> Result<Self, CliError> {
        Self::load_manifest_backed_package(project_root, None)
    }

    fn load_dependency_package(
        project_root: &Path,
        expected_alias: Option<&str>,
    ) -> Result<Self, CliError> {
        Self::load_manifest_backed_package(project_root, expected_alias)
    }

    fn load_manifest_backed_package(
        project_root: &Path,
        expected_alias: Option<&str>,
    ) -> Result<Self, CliError> {
        let config = load_required_project_file_config(project_root)?;
        let config_path = project_root.join("loz.toml");
        let name = config
            .project
            .as_ref()
            .and_then(|project| project.name.clone())
            .or_else(|| {
                project_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "app".to_string());

        if let Some(alias) = expected_alias {
            if name != alias {
                return Err(CliError::new(format!(
                    "dependency '{}' declares package name '{}' in '{}'",
                    alias,
                    name,
                    config_path.display()
                )));
            }
        }

        let main_relative = resolve_package_main_relative(project_root, &config, expected_alias)?;
        let main_path =
            canonicalize_existing_file(&project_root.join(&main_relative)).map_err(|_| {
                let label = expected_alias.unwrap_or(&name);
                CliError::new(format!(
                    "package '{}' main file '{}' does not exist",
                    label,
                    main_relative.display()
                ))
            })?;

        Ok(Self {
            name,
            root_dir: project_root.to_path_buf(),
            main_relative,
            main_path,
            dependency_specs: config.dependencies,
            dependencies: HashMap::new(),
        })
    }

    fn resolve_direct_dependencies(&self) -> Result<Vec<ResolvedDependency>, CliError> {
        let mut dependencies = Vec::new();
        for (alias, dependency) in &self.dependency_specs {
            let raw_path = PathBuf::from(&dependency.path);
            let dependency_dir = resolve_project_relative_path_buf(&self.root_dir, &raw_path);
            if !dependency_dir.exists() {
                return Err(CliError::new(format!(
                    "dependency '{}' path '{}' does not exist",
                    alias, dependency.path
                )));
            }

            let dependency_root = canonicalize_existing_dir(&dependency_dir)?;
            let manifest_path = dependency_root.join("loz.toml");
            if !manifest_path.is_file() {
                return Err(CliError::new(format!(
                    "dependency '{}' is missing loz.toml",
                    alias
                )));
            }

            let dependency_package =
                PackageInfo::load_dependency_package(&dependency_root, Some(alias))?;
            dependencies.push(ResolvedDependency {
                alias: alias.clone(),
                path_text: dependency.path.clone(),
                package_root: dependency_root,
                main_relative: dependency_package.main_relative,
                main_path: dependency_package.main_path,
            });
        }

        dependencies.sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(dependencies)
    }
}

fn canonicalize_existing_file(path: &Path) -> Result<PathBuf, CliError> {
    fs::canonicalize(path).map_err(|error| {
        CliError::new(format!(
            "failed to resolve source file '{}': {error}",
            path.display()
        ))
    })
}

fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf, CliError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        CliError::new(format!(
            "failed to resolve directory '{}': {error}",
            path.display()
        ))
    })?;

    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(CliError::new(format!(
            "path '{}' is not a directory",
            canonical.display()
        )))
    }
}

fn resolve_package_main_relative(
    project_root: &Path,
    config: &ProjectFileConfig,
    package_label: Option<&str>,
) -> Result<PathBuf, CliError> {
    if let Some(main) = config
        .project
        .as_ref()
        .and_then(|project| project.main.as_deref())
    {
        return Ok(PathBuf::from(main));
    }

    for candidate in ["src/lib.loz", "src/main.loz"] {
        if project_root.join(candidate).is_file() {
            return Ok(PathBuf::from(candidate));
        }
    }

    let label = package_label.unwrap_or_else(|| {
        project_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("app")
    });
    Err(CliError::new(format!(
        "package '{}' is missing a main source file; expected [project].main, src/lib.loz, or src/main.loz",
        label
    )))
}

fn resolve_project_relative_path_buf(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn find_tool(
    env_override: Option<&'static str>,
    candidates: &[&str],
    label: &str,
) -> Result<ResolvedTool, String> {
    if let Some(env_key) = env_override {
        if let Some(value) = env::var_os(env_key).filter(|value| !value.is_empty()) {
            let override_path = PathBuf::from(value);
            if override_path.is_file() {
                return Ok(ResolvedTool {
                    name: label.to_string(),
                    path: override_path,
                    source: ToolSource::EnvOverride(env_key),
                });
            }

            return Err(format!(
                "{} was not found at '{}'. Fix {} or install {} on PATH.",
                label,
                override_path.display(),
                env_key,
                label
            ));
        }
    }

    for candidate in candidates {
        if let Some(path) = find_tool_on_path(candidate) {
            return Ok(ResolvedTool {
                name: label.to_string(),
                path,
                source: ToolSource::Path,
            });
        }
    }

    Err(match env_override {
        Some(env_key) => format!(
            "{} was not found on PATH. Install {} or set {}.",
            label, label, env_key
        ),
        None => format!("{label} was not found on PATH. Install {label}."),
    })
}

fn find_python_tool(configured_python: Option<&str>) -> Result<ResolvedTool, String> {
    if let Some(python) = configured_python {
        return find_tool(Some("LOZ_PYTHON_PATH"), &[python], "python");
    }

    find_tool(Some("LOZ_PYTHON_PATH"), &["python3", "python"], "python")
}

fn find_tool_on_path(command: &str) -> Option<PathBuf> {
    if command_contains_path_components(command) {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }

    let path_value = env::var_os("PATH")?;
    let windows_exts = windows_path_extensions();
    let has_extension = Path::new(command).extension().is_some();

    for directory in env::split_paths(&path_value) {
        let direct = directory.join(command);
        if direct.is_file() {
            return Some(direct);
        }

        if cfg!(target_os = "windows") && !has_extension {
            for extension in &windows_exts {
                let candidate = directory.join(format!("{command}{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

fn windows_path_extensions() -> Vec<String> {
    env::var("PATHEXT")
        .ok()
        .map(|value| {
            value
                .split(';')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| {
            vec![
                ".exe".to_string(),
                ".cmd".to_string(),
                ".bat".to_string(),
                ".com".to_string(),
            ]
        })
}

fn command_contains_path_components(command: &str) -> bool {
    Path::new(command).components().count() > 1 || command.contains('/')
}

fn is_builtin_module(name: &str) -> bool {
    BUILTIN_MODULES.contains(&name)
}

fn build_native_executable(
    source_path: &Path,
    project_root: &Path,
    program: &Program,
) -> Result<(), CliError> {
    let platform = BuildPlatform::current();
    let ir = generate_llvm_ir(program)
        .map_err(|error| CliError::with_file_path(error.to_string(), source_path))?;
    let clang = find_tool(Some("LOZ_CLANG_PATH"), &[platform.default_clang], "clang")
        .map_err(CliError::new)?;
    let llc =
        find_tool(Some("LOZ_LLC_PATH"), &[platform.default_llc], "llc").map_err(CliError::new)?;
    let cargo = find_tool(Some("LOZ_CARGO_PATH"), &["cargo"], "cargo").map_err(CliError::new)?;
    let runtime_link = prepare_loz_runtime_link_artifacts(&cargo.path)?;
    let source_stem = source_path.file_stem().ok_or_else(|| {
        CliError::new(format!(
            "failed to determine output name from source path '{}'",
            source_path.display()
        ))
    })?;
    let source_stem = source_stem.to_string_lossy();

    let output_dir = project_root.join("output");
    fs::create_dir_all(&output_dir).map_err(|error| {
        CliError::new(format!(
            "failed to create output directory '{}': {error}",
            output_dir.display()
        ))
    })?;

    let build_token = unique_build_token();
    let temp_build_dir = TempBuildDir::create("loz_build", &build_token)?;
    let temp_ll_path = temp_build_dir.path().join(format!("{source_stem}.ll"));
    let temp_object_path = temp_build_dir
        .path()
        .join(platform.object_file_name(&source_stem));
    let temp_executable_path =
        output_dir.join(platform.staged_executable_name(&source_stem, &build_token));
    let final_ll_path = output_dir.join(source_stem.as_ref()).with_extension("ll");
    let final_executable_path = output_dir.join(platform.executable_name(&source_stem));

    fs::write(&temp_ll_path, ir).map_err(|error| {
        CliError::new(format!(
            "failed to write LLVM IR file '{}': {error}",
            temp_ll_path.display()
        ))
    })?;

    run_command(
        "llc",
        Command::new(&llc.path)
            .arg("-filetype=obj")
            .arg(&temp_ll_path)
            .arg("-o")
            .arg(&temp_object_path),
    )?;

    let mut clang = build_clang_link_command(
        platform,
        &clang.path,
        &temp_object_path,
        &runtime_link,
        &temp_executable_path,
    );
    run_command("clang", &mut clang)?;

    atomically_replace(&temp_ll_path, &final_ll_path)?;
    atomically_replace(&temp_executable_path, &final_executable_path)?;

    if temp_object_path.exists() {
        fs::remove_file(&temp_object_path).map_err(|error| {
            CliError::new(format!(
                "failed to remove intermediate object file '{}': {error}",
                temp_object_path.display()
            ))
        })?;
    }

    println!(
        "Built native executable: '{}' and LLVM IR: '{}'",
        final_executable_path.display(),
        final_ll_path.display()
    );

    Ok(())
}

fn build_clang_link_command<'a>(
    platform: BuildPlatform,
    clang_path: &'a Path,
    object_path: &'a Path,
    runtime_link: &'a RuntimeLinkArtifacts,
    executable_path: &'a Path,
) -> Command {
    let mut args = vec![
        object_path.to_string_lossy().into_owned(),
        runtime_link.library_path.to_string_lossy().into_owned(),
    ];

    if platform.os == PlatformOs::Linux {
        args.extend([
            "-no-pie".to_string(),
            "-L/usr/lib/x86_64-linux-gnu".to_string(),
            "-L/lib/x86_64-linux-gnu".to_string(),
        ]);
    }

    args.push("-o".to_string());
    args.push(executable_path.to_string_lossy().into_owned());
    args.extend(runtime_link.native_static_libs.iter().cloned());

    sanitize_clang_link_args(platform, &mut args);
    debug_assert!(
        !args.iter().any(|arg| should_skip_link_arg(platform, arg)),
        "linux clang link args must not contain explicit -lc: {args:?}"
    );

    let mut command = Command::new(clang_path);
    command.args(&args);

    command
}

fn sanitize_clang_link_args(platform: BuildPlatform, args: &mut Vec<String>) {
    args.retain(|arg| !should_skip_link_arg(platform, arg));
}

fn should_skip_link_arg(platform: BuildPlatform, arg: &str) -> bool {
    platform.os == PlatformOs::Linux && arg.trim() == "-lc"
}

fn prepare_loz_runtime_link_artifacts(cargo_path: &Path) -> Result<RuntimeLinkArtifacts, CliError> {
    let profile_dir = current_profile_dir()?;
    let workspace_root = find_compiler_workspace_root()?;
    let is_release = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "release")
        .unwrap_or(false);

    let mut cargo = Command::new(cargo_path);
    cargo.current_dir(&workspace_root);
    cargo.arg("rustc").arg("-p").arg("loz_runtime");
    if is_release {
        cargo.arg("--release");
    }
    cargo.arg("--").arg("--print").arg("native-static-libs");

    let output = cargo
        .output()
        .map_err(|error| CliError::new(format!("failed to build loz_runtime: {error}")))?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::new(format!(
            "failed to build loz_runtime for native linking.\nstdout:\n{}\nstderr:\n{}",
            stdout.trim_end(),
            stderr.trim_end(),
        )));
    }

    let combined_output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let library_path = profile_dir.join(static_runtime_library_name());
    if !library_path.exists() {
        return Err(CliError::new(format!(
            "native runtime library was not produced at '{}'",
            library_path.display()
        )));
    }

    Ok(RuntimeLinkArtifacts {
        library_path,
        native_static_libs: parse_native_static_libs(&combined_output),
    })
}

fn current_profile_dir() -> Result<PathBuf, CliError> {
    let current_executable = std::env::current_exe().map_err(|error| {
        CliError::new(format!("failed to locate current loz executable: {error}"))
    })?;

    current_executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            CliError::new(format!(
                "failed to determine build profile directory from '{}'",
                current_executable.display()
            ))
        })
}

fn find_compiler_workspace_root() -> Result<PathBuf, CliError> {
    let current_executable = std::env::current_exe().map_err(|error| {
        CliError::new(format!("failed to locate current loz executable: {error}"))
    })?;

    if let Some(root) = find_workspace_root_from_path(&current_executable) {
        return Ok(root);
    }

    let fallback_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let Some(root) = find_workspace_root_from_path(&fallback_root) {
        return Ok(root);
    }

    Err(CliError::new(
        "failed to determine Loz compiler workspace root for native runtime linking",
    ))
}

fn find_workspace_root_from_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        let cargo_toml = ancestor.join("Cargo.toml");
        let runtime_manifest = ancestor.join("crates/loz_runtime/Cargo.toml");
        if cargo_toml.is_file() && runtime_manifest.is_file() {
            Some(ancestor.to_path_buf())
        } else {
            None
        }
    })
}

fn static_runtime_library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "loz_runtime.lib"
    } else {
        "libloz_runtime.a"
    }
}

fn parse_native_static_libs(output: &str) -> Vec<String> {
    output
        .lines()
        .find_map(|line| line.split_once("native-static-libs:").map(|(_, libs)| libs))
        .map(|libs| libs.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn run_command(program: &str, command: &mut Command) -> Result<(), CliError> {
    let command_line = format_command(command);
    let output = command.output().map_err(|error| {
        CliError::new(format!(
            "failed to run {program}: {error}\ncommand:\n{command_line}"
        ))
    })?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    Err(CliError::new(format!(
        "{program} failed with status {}.\ncommand:\n{}\nstdout:\n{}\nstderr:\n{}",
        output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string()),
        command_line,
        stdout.trim_end(),
        stderr.trim_end(),
    )))
}

fn format_command(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|argument| quote_command_argument(&argument.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_command_argument(argument: &str) -> String {
    if argument.is_empty()
        || argument
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '"' | '\''))
    {
        format!("\"{}\"", argument.replace('"', "\\\""))
    } else {
        argument.to_string()
    }
}

fn find_project_root_from_source(source_path: &Path) -> Option<PathBuf> {
    let start_dir = source_path.parent().unwrap_or(source_path);
    find_project_root_from_dir(start_dir)
}

fn find_project_root_from_dir(start_dir: &Path) -> Option<PathBuf> {
    start_dir.ancestors().find_map(|ancestor| {
        let config_path = ancestor.join("loz.toml");
        if config_path.is_file() {
            Some(ancestor.to_path_buf())
        } else {
            None
        }
    })
}

fn resolve_path_from_cwd(current_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    }
}

fn resolve_project_relative_path(project_root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}

fn load_project_file_config(project_root: &Path) -> Result<ProjectFileConfig, CliError> {
    let config_path = project_root.join("loz.toml");
    if !config_path.is_file() {
        return Ok(ProjectFileConfig::default());
    }

    load_project_file_config_from_path(&config_path)
}

fn load_required_project_file_config(project_root: &Path) -> Result<ProjectFileConfig, CliError> {
    let config_path = project_root.join("loz.toml");
    if !config_path.is_file() {
        return Err(CliError::new(format!(
            "expected loz.toml at project root '{}'",
            project_root.display()
        )));
    }

    load_project_file_config_from_path(&config_path)
}

fn load_project_file_config_from_path(path: &Path) -> Result<ProjectFileConfig, CliError> {
    let text = fs::read_to_string(path).map_err(|error| {
        CliError::new(format!(
            "failed to read project config '{}': {error}",
            path.display()
        ))
    })?;
    toml::from_str(&text).map_err(|error| {
        CliError::new(format!("invalid loz.toml at '{}': {error}", path.display()))
    })
}

fn load_dotenv_values(project_root: &Path) -> Result<HashMap<String, String>, CliError> {
    let dotenv_path = project_root.join(".env");
    if !dotenv_path.is_file() {
        return Ok(HashMap::new());
    }

    let mut values = HashMap::new();
    let entries = dotenvy::from_path_iter(&dotenv_path).map_err(|error| {
        CliError::new(format!(
            "failed to parse .env file '{}': {error}",
            dotenv_path.display()
        ))
    })?;

    for entry in entries {
        let (key, value) = entry.map_err(|error| {
            CliError::new(format!(
                "failed to parse .env file '{}': {error}",
                dotenv_path.display()
            ))
        })?;
        values.insert(key, value);
    }

    Ok(values)
}

fn atomically_replace(source: &Path, destination: &Path) -> Result<(), CliError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| {
            CliError::new(format!(
                "failed to replace existing output '{}': {error}",
                destination.display()
            ))
        })?;
    }

    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(18) => {
            fs::copy(source, destination).map_err(|copy_error| {
                CliError::new(format!(
                    "failed to copy build artifact '{}' to '{}': {copy_error}",
                    source.display(),
                    destination.display()
                ))
            })?;
            fs::remove_file(source).map_err(|remove_error| {
                CliError::new(format!(
                    "failed to remove temporary build artifact '{}': {remove_error}",
                    source.display()
                ))
            })?;
            Ok(())
        }
        Err(error) => Err(CliError::new(format!(
            "failed to move build artifact '{}' to '{}': {error}",
            source.display(),
            destination.display()
        ))),
    }
}

fn unique_build_token() -> String {
    format!(
        "{}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        BUILD_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug)]
struct CliError {
    message: String,
    diagnostic: Option<Diagnostic>,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            diagnostic: None,
        }
    }

    fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        Self {
            message: diagnostic.message.clone(),
            diagnostic: Some(diagnostic),
        }
    }

    fn with_file_path(message: impl Into<String>, file_path: &Path) -> Self {
        Self::from_diagnostic(
            Diagnostic::error(message.into()).with_file_path(file_path.display().to_string()),
        )
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(diagnostic) = &self.diagnostic {
            write!(f, "{}", diagnostic.render())
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for CliError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramCommandKind {
    Run,
    Check,
    LlvmIr,
    Build,
}

impl ProgramCommandKind {
    fn parse(text: &str) -> Result<Self, CliError> {
        match text {
            "run" => Ok(Self::Run),
            "check" => Ok(Self::Check),
            "llvm-ir" => Ok(Self::LlvmIr),
            "build" => Ok(Self::Build),
            other => Err(CliError::new(format!(
                "unknown command '{other}'. usage: {}",
                cli_usage()
            ))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CliCommand {
    Program(ProgramCommandInvocation),
    Agent(AgentCommandInvocation),
    Workflow(WorkflowCommandInvocation),
    Deps,
    Doctor,
    Init { project_path: PathBuf },
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
struct ProgramCommandInvocation {
    kind: ProgramCommandKind,
    source_argument: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum AgentCommandInvocation {
    List { source_argument: Option<PathBuf> },
    Run { raw_arguments: Vec<String> },
}

impl AgentCommandInvocation {
    fn parse(subcommand: &str, arguments: Vec<String>) -> Result<Self, CliError> {
        match subcommand {
            "list" => {
                if arguments.len() > 1 {
                    return Err(CliError::new(agent_list_usage()));
                }

                Ok(Self::List {
                    source_argument: arguments.first().map(PathBuf::from),
                })
            }
            "run" => Ok(Self::Run {
                raw_arguments: arguments,
            }),
            other => Err(CliError::new(format!(
                "unknown agent command '{other}'. usage: {}",
                agent_usage()
            ))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum WorkflowCommandInvocation {
    List { source_argument: Option<PathBuf> },
    Run { raw_arguments: Vec<String> },
}

impl WorkflowCommandInvocation {
    fn parse(subcommand: &str, arguments: Vec<String>) -> Result<Self, CliError> {
        match subcommand {
            "list" => {
                if arguments.len() > 1 {
                    return Err(CliError::new(workflow_list_usage()));
                }

                Ok(Self::List {
                    source_argument: arguments.first().map(PathBuf::from),
                })
            }
            "run" => Ok(Self::Run {
                raw_arguments: arguments,
            }),
            other => Err(CliError::new(format!(
                "unknown workflow command '{other}'. usage: {}",
                workflow_usage()
            ))),
        }
    }
}

struct CliInvocation {
    command: CliCommand,
}

struct RuntimeLinkArtifacts {
    library_path: PathBuf,
    native_static_libs: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectFileConfig {
    project: Option<ProjectSection>,
    #[serde(default)]
    dependencies: HashMap<String, DependencySection>,
    python: Option<PythonSection>,
    llm: Option<LlmSection>,
    ollama: Option<OllamaSection>,
    github: Option<GitHubSection>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProjectSection {
    name: Option<String>,
    version: Option<String>,
    main: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DependencySection {
    path: String,
}

#[derive(Debug, Deserialize)]
struct PythonSection {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlmSection {
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaSection {
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubSection {
    token_env: Option<String>,
    models_base_url: Option<String>,
}

#[derive(Debug)]
struct ProjectContext {
    project_root: PathBuf,
    source_path: PathBuf,
    config: ProjectFileConfig,
    dotenv_values: HashMap<String, String>,
}

impl ProjectContext {
    fn apply_runtime_environment(&self) -> Result<(), CliError> {
        self.set_env_if_absent("LOZ_PYTHON_PATH", self.python_path_from_config().as_deref())?;
        self.set_env_if_absent(
            "LOZ_LLM_PROVIDER",
            self.llm_provider_from_config().as_deref(),
        )?;
        self.set_env_if_absent("LOZ_MODEL", self.llm_model_from_config().as_deref())?;
        self.set_env_if_absent(
            "LOZ_OLLAMA_BASE_URL",
            self.ollama_base_url_from_config().as_deref(),
        )?;
        self.set_env_if_absent(
            "LOZ_GITHUB_MODELS_BASE_URL",
            self.github_models_base_url_from_config().as_deref(),
        )?;

        let token_env_name = self.github_token_env_name();
        self.set_env_if_absent("LOZ_GITHUB_TOKEN_ENV", Some(token_env_name.as_str()))?;
        self.propagate_github_token(&token_env_name)?;
        Ok(())
    }

    fn set_env_if_absent(&self, key: &str, config_value: Option<&str>) -> Result<(), CliError> {
        if env::var_os(key).is_some() {
            return Ok(());
        }

        if let Some(value) = self.dotenv_values.get(key) {
            unsafe {
                env::set_var(key, value);
            }
            return Ok(());
        }

        if let Some(value) = config_value {
            unsafe {
                env::set_var(key, value);
            }
        }

        Ok(())
    }

    fn propagate_github_token(&self, token_env_name: &str) -> Result<(), CliError> {
        if env::var_os(token_env_name).is_none() {
            if let Some(value) = self.dotenv_values.get(token_env_name) {
                unsafe {
                    env::set_var(token_env_name, value);
                }
            }
        }

        if env::var_os("GITHUB_TOKEN").is_none() {
            if let Some(value) = env::var_os(token_env_name) {
                unsafe {
                    env::set_var("GITHUB_TOKEN", value);
                }
            }
        }

        Ok(())
    }

    fn github_token_env_name(&self) -> String {
        env::var("LOZ_GITHUB_TOKEN_ENV")
            .ok()
            .or_else(|| self.dotenv_values.get("LOZ_GITHUB_TOKEN_ENV").cloned())
            .or_else(|| {
                self.config
                    .github
                    .as_ref()
                    .and_then(|github| github.token_env.clone())
            })
            .unwrap_or_else(|| "GITHUB_TOKEN".to_string())
    }

    fn python_path_from_config(&self) -> Option<String> {
        self.config
            .python
            .as_ref()
            .and_then(|python| python.path.clone())
    }

    fn llm_provider_from_config(&self) -> Option<String> {
        self.config
            .llm
            .as_ref()
            .and_then(|llm| llm.provider.clone())
    }

    fn llm_model_from_config(&self) -> Option<String> {
        self.config.llm.as_ref().and_then(|llm| llm.model.clone())
    }

    fn ollama_base_url_from_config(&self) -> Option<String> {
        self.config
            .ollama
            .as_ref()
            .and_then(|ollama| ollama.base_url.clone())
    }

    fn github_models_base_url_from_config(&self) -> Option<String> {
        self.config
            .github
            .as_ref()
            .and_then(|github| github.models_base_url.clone())
    }
}

struct WorkingDirectoryGuard {
    previous_directory: PathBuf,
}

impl WorkingDirectoryGuard {
    fn change_to(target_directory: &Path) -> Result<Self, CliError> {
        let previous_directory = env::current_dir()
            .map_err(|error| CliError::new(format!("failed to read current directory: {error}")))?;
        if previous_directory == target_directory {
            return Ok(Self { previous_directory });
        }

        env::set_current_dir(target_directory).map_err(|error| {
            CliError::new(format!(
                "failed to change working directory to '{}': {error}",
                target_directory.display()
            ))
        })?;

        Ok(Self { previous_directory })
    }
}

impl Drop for WorkingDirectoryGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.previous_directory);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use loz_codegen::{RuntimeValue, execute, generate_llvm_ir};
    use loz_parser::parse_program;
    use loz_semantic::analyze;

    use super::{
        AgentCommandInvocation, BuildPlatform, CliCommand, DiscoveredAgent, DiscoveredTask,
        DiscoveredWorkflow, DiscoveredWorkflowStep, DoctorSeverity, PackageInfo,
        ProgramCommandInvocation, ProgramCommandKind, ResolvedDependency, RuntimeLinkArtifacts,
        ToolSource, ambiguous_agent_selection_error, build_clang_link_command, collect_agents,
        collect_doctor_report, collect_workflows, find_compiler_workspace_root, find_python_tool,
        find_tool, format_task_result, infer_project_name, init_main_source, load_checked_program,
        load_program_from_entry, load_project_file_config, load_project_file_config_from_path,
        parse_task_argument, parse_task_arguments, render_agent_list, render_dependencies,
        render_doctor_report, render_workflow_list, resolve_agent_task_selection,
        resolve_project_context, resolve_project_relative_path_buf, resolve_workflow_selection,
        run_agent_task, run_init, run_workflow, split_agent_run_source_argument,
        split_workflow_run_source_argument, type_name_label,
    };

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    const TEST_ENV_KEYS: &[&str] = &[
        "LOZ_PYTHON_PATH",
        "LOZ_LLM_PROVIDER",
        "LOZ_LLM_MOCK_RESPONSE",
        "LOZ_MODEL",
        "LOZ_OLLAMA_BASE_URL",
        "LOZ_CLANG_PATH",
        "LOZ_LLC_PATH",
        "LOZ_CARGO_PATH",
        "LOZ_GITHUB_MODELS_BASE_URL",
        "LOZ_GITHUB_TOKEN_ENV",
        "GITHUB_TOKEN",
        "CUSTOM_GITHUB_TOKEN",
        "PATH",
    ];

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}_{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_process_state<T>(
        current_dir: &Path,
        updates: &[(&str, Option<&str>)],
        test: impl FnOnce() -> T,
    ) -> T {
        let _guard = TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous_dir = std::env::current_dir().unwrap();
        let mut previous_values = HashMap::new();
        for key in TEST_ENV_KEYS {
            previous_values.insert((*key).to_string(), std::env::var(key).ok());
        }
        for (key, _) in updates {
            previous_values
                .entry((*key).to_string())
                .or_insert_with(|| std::env::var(key).ok());
        }

        std::env::set_current_dir(current_dir).unwrap();
        for (key, value) in updates {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }

        let result = test();

        std::env::set_current_dir(previous_dir).unwrap();
        for (key, value) in previous_values {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }

        result
    }

    fn python_is_available() -> bool {
        find_python_tool(None).is_ok()
    }

    fn parse_checked_program(source: &str) -> loz_ast::Program {
        let program = parse_program(loz_lexer::tokenize(source).unwrap()).unwrap();
        analyze(&program).unwrap();
        program
    }

    #[test]
    fn loads_imported_module_program() {
        let dir = make_temp_dir("loz_modules");
        let math_path = dir.join("math.loz");
        let main_path = dir.join("main.loz");

        fs::write(
            &math_path,
            r#"module math;

func add(a: Int, b: Int) -> Int {
    return a + b;
}
"#,
        )
        .unwrap();

        fs::write(
            &main_path,
            r#"module main;

import math;

func main() -> Int {
    return add(10, 20);
}
"#,
        )
        .unwrap();

        let program = load_program_from_entry(&main_path).unwrap();
        analyze(&program).unwrap();
        let result = execute(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(30));
    }

    #[test]
    fn rejects_duplicate_imports_in_same_file() {
        let dir = make_temp_dir("loz_duplicate_imports");
        let math_path = dir.join("math.loz");
        let main_path = dir.join("main.loz");

        fs::write(
            &math_path,
            r#"module math;

func add(a: Int, b: Int) -> Int {
    return a + b;
}
"#,
        )
        .unwrap();

        fs::write(
            &main_path,
            r#"module main;
import math;
import math;

func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let error = load_program_from_entry(&main_path).unwrap_err();
        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn parses_dependencies_from_loz_toml() {
        let dir = make_temp_dir("loz_dep_toml");
        let config_path = dir.join("loz.toml");
        fs::write(
            &config_path,
            r#"[project]
name = "support-agent"
version = "0.1.0"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();

        let config = load_project_file_config_from_path(&config_path).unwrap();
        assert_eq!(
            config
                .dependencies
                .get("text_utils")
                .map(|dependency| dependency.path.as_str()),
            Some("./packages/text_utils")
        );
    }

    #[test]
    fn build_platform_suffixes_match_expected_shapes() {
        assert_eq!(BuildPlatform::linux().executable_name("main"), "main");
        assert_eq!(BuildPlatform::linux().object_file_name("main"), "main.o");
        assert_eq!(BuildPlatform::macos().object_file_name("main"), "main.o");
        assert_eq!(BuildPlatform::windows().executable_name("main"), "main.exe");
        assert_eq!(
            BuildPlatform::windows().object_file_name("main"),
            "main.obj"
        );
    }

    #[test]
    fn build_link_command_is_platform_aware() {
        let runtime_link = RuntimeLinkArtifacts {
            library_path: PathBuf::from("libloz_runtime.a"),
            native_static_libs: vec![
                "-lgcc_s".to_string(),
                "-lutil".to_string(),
                "-lrt".to_string(),
                "-lpthread".to_string(),
                "-lm".to_string(),
                "-ldl".to_string(),
                " -lc ".to_string(),
            ],
        };

        let linux = build_clang_link_command(
            BuildPlatform::linux(),
            Path::new("clang"),
            Path::new("main.o"),
            &runtime_link,
            Path::new("output/main"),
        );
        let linux_args: Vec<String> = linux
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(linux_args.iter().any(|arg| arg == "-no-pie"));
        assert!(
            linux_args
                .iter()
                .any(|arg| arg == "-L/usr/lib/x86_64-linux-gnu")
        );
        assert!(
            linux_args
                .iter()
                .any(|arg| arg == "-L/lib/x86_64-linux-gnu")
        );
        assert!(linux_args.iter().any(|arg| arg == "-ldl"));
        assert!(linux_args.iter().any(|arg| arg == "-lm"));
        assert!(linux_args.iter().any(|arg| arg == "-lpthread"));
        assert!(!linux_args.iter().any(|arg| arg == "-lc"));
        assert!(!linux_args.iter().any(|arg| arg.trim() == "-lc"));

        let macos = build_clang_link_command(
            BuildPlatform::macos(),
            Path::new("clang"),
            Path::new("main.o"),
            &runtime_link,
            Path::new("output/main"),
        );
        let macos_args: Vec<String> = macos
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(!macos_args.iter().any(|arg| arg == "-no-pie"));
        assert!(
            !macos_args
                .iter()
                .any(|arg| arg == "-L/usr/lib/x86_64-linux-gnu")
        );
        assert!(
            !macos_args
                .iter()
                .any(|arg| arg == "-L/lib/x86_64-linux-gnu")
        );
        assert!(macos_args.iter().any(|arg| arg.trim() == "-lc"));

        let windows = build_clang_link_command(
            BuildPlatform::windows(),
            Path::new("clang.exe"),
            Path::new("main.obj"),
            &runtime_link,
            Path::new("output/main.exe"),
        );
        let windows_args: Vec<String> = windows
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(!windows_args.iter().any(|arg| arg == "-no-pie"));
        assert!(
            !windows_args
                .iter()
                .any(|arg| arg == "-L/usr/lib/x86_64-linux-gnu")
        );
        assert!(
            !windows_args
                .iter()
                .any(|arg| arg == "-L/lib/x86_64-linux-gnu")
        );
        assert!(windows_args.iter().any(|arg| arg.trim() == "-lc"));
        assert_eq!(windows.get_program().to_string_lossy(), "clang.exe");
    }

    #[test]
    fn find_tool_respects_env_override() {
        let dir = make_temp_dir("loz_tool_override");
        let override_path = dir.join("clang-custom");
        fs::write(&override_path, "stub").unwrap();

        let resolved = with_process_state(
            &dir,
            &[
                ("LOZ_CLANG_PATH", Some(override_path.to_str().unwrap())),
                ("PATH", Some("")),
            ],
            || find_tool(Some("LOZ_CLANG_PATH"), &["clang"], "clang").unwrap(),
        );

        assert_eq!(resolved.path, override_path);
        assert_eq!(resolved.source, ToolSource::EnvOverride("LOZ_CLANG_PATH"));
    }

    #[test]
    fn missing_tool_reports_clear_error() {
        let dir = make_temp_dir("loz_tool_missing");
        let message =
            with_process_state(&dir, &[("LOZ_LLC_PATH", None), ("PATH", Some(""))], || {
                find_tool(Some("LOZ_LLC_PATH"), &["llc"], "llc").unwrap_err()
            });

        assert!(message.contains("llc was not found on PATH"));
        assert!(message.contains("LOZ_LLC_PATH"));
    }

    #[test]
    fn python_tool_falls_back_to_python_when_python3_is_missing() {
        let dir = make_temp_dir("loz_python_fallback");
        let python_path = dir.join("python");
        fs::write(&python_path, "stub").unwrap();

        let resolved = with_process_state(
            &dir,
            &[
                ("LOZ_PYTHON_PATH", None),
                ("PATH", Some(dir.to_str().unwrap())),
            ],
            || find_python_tool(None).unwrap(),
        );

        assert_eq!(resolved.path, python_path);
        assert_eq!(resolved.source, ToolSource::Path);
    }

    #[test]
    fn resolves_dependency_main_with_lib_fallback() {
        let dir = make_temp_dir("loz_dep_main_fallback");
        let package_dir = dir.join("text_utils");
        fs::create_dir_all(package_dir.join("src")).unwrap();
        fs::write(
            package_dir.join("loz.toml"),
            r#"[project]
name = "text_utils"
version = "0.1.0"
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("src/lib.loz"),
            r#"module text_utils;

func title() -> Text {
    return "Hello";
}
"#,
        )
        .unwrap();

        let package =
            PackageInfo::load_dependency_package(&package_dir, Some("text_utils")).unwrap();
        assert_eq!(package.main_relative, PathBuf::from("src/lib.loz"));
    }

    #[test]
    fn rejects_missing_dependency_path() {
        let dir = make_temp_dir("loz_missing_dep_path");
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
name = "demo"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/main.loz"),
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let package = PackageInfo::load_root_package(&dir).unwrap();
        let error = package.resolve_direct_dependencies().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("dependency 'text_utils' path './packages/text_utils' does not exist")
        );
    }

    #[test]
    fn resolves_parent_relative_dependency_paths() {
        let dir = make_temp_dir("loz_dep_parent_relative");
        let project_dir = dir.join("app");
        let package_dir = dir.join("text_utils");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::create_dir_all(package_dir.join("src")).unwrap();

        fs::write(
            project_dir.join("loz.toml"),
            r#"[project]
name = "app"
main = "src/main.loz"

[dependencies]
text_utils = { path = "../text_utils" }
"#,
        )
        .unwrap();
        fs::write(
            project_dir.join("src/main.loz"),
            "func main() -> i32 { return 0; }\n",
        )
        .unwrap();
        fs::write(
            package_dir.join("loz.toml"),
            r#"[project]
name = "text_utils"
main = "src/lib.loz"
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("src/lib.loz"),
            "module text_utils;\nfunc title() -> Text { return \"Hello\"; }\n",
        )
        .unwrap();

        let package = PackageInfo::load_root_package(&project_dir).unwrap();
        let dependencies = package.resolve_direct_dependencies().unwrap();

        assert_eq!(dependencies.len(), 1);
        assert_eq!(
            dependencies[0].package_root,
            fs::canonicalize(&package_dir).unwrap()
        );
        assert_eq!(
            resolve_project_relative_path_buf(&project_dir, Path::new("../text_utils")),
            project_dir.join("../text_utils")
        );
    }

    #[test]
    fn rejects_missing_dependency_manifest() {
        let dir = make_temp_dir("loz_missing_dep_manifest");
        fs::create_dir_all(dir.join("packages/text_utils")).unwrap();
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
name = "demo"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/main.loz"),
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let package = PackageInfo::load_root_package(&dir).unwrap();
        let error = package.resolve_direct_dependencies().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("dependency 'text_utils' is missing loz.toml")
        );
    }

    #[test]
    fn detects_circular_dependencies() {
        let dir = make_temp_dir("loz_circular_deps");
        let app_dir = dir.join("app");
        let dep_dir = app_dir.join("packages/text_utils");
        let dep_nested_dir = dep_dir.join("packages/app");
        fs::create_dir_all(app_dir.join("src")).unwrap();
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        fs::create_dir_all(dep_nested_dir.join("src")).unwrap();

        fs::write(
            app_dir.join("loz.toml"),
            r#"[project]
name = "app"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();
        fs::write(
            app_dir.join("src/main.loz"),
            "import text_utils;\nfunc main() -> Int { return 0; }\n",
        )
        .unwrap();

        fs::write(
            dep_dir.join("loz.toml"),
            r#"[project]
name = "text_utils"
main = "src/lib.loz"

[dependencies]
app = { path = "./packages/app" }
"#,
        )
        .unwrap();
        fs::write(
            dep_dir.join("src/lib.loz"),
            "module text_utils;\nfunc title() -> Text { return \"x\"; }\n",
        )
        .unwrap();

        fs::write(
            dep_nested_dir.join("loz.toml"),
            r#"[project]
name = "app"
main = "src/lib.loz"

[dependencies]
text_utils = { path = "../../" }
"#,
        )
        .unwrap();
        fs::write(
            dep_nested_dir.join("src/lib.loz"),
            "module app;\nfunc title() -> Text { return \"x\"; }\n",
        )
        .unwrap();

        let error = load_program_from_entry(&app_dir.join("src/main.loz")).unwrap_err();
        assert!(error.to_string().contains("circular dependency detected"));
    }

    #[test]
    fn loads_qualified_local_module_program() {
        let dir = make_temp_dir("loz_local_module_qualified");
        let users_path = dir.join("users.loz");
        let main_path = dir.join("main.loz");

        fs::write(
            &users_path,
            r#"module users;

func get_name() -> Text {
    return "Ahmed";
}
"#,
        )
        .unwrap();

        fs::write(
            &main_path,
            r#"import users;

func main() -> Text {
    return users.get_name();
}
"#,
        )
        .unwrap();

        let program = load_program_from_entry(&main_path).unwrap();
        analyze(&program).unwrap();
        let result = execute(&program).unwrap();

        assert_eq!(result, RuntimeValue::Text("Ahmed".to_string()));
    }

    #[test]
    fn loads_local_path_dependency_program() {
        let dir = make_temp_dir("loz_dependency_import");
        let package_dir = dir.join("packages/text_utils");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(package_dir.join("src")).unwrap();

        fs::write(
            dir.join("loz.toml"),
            r#"[project]
name = "demo"
version = "0.1.0"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("loz.toml"),
            r#"[project]
name = "text_utils"
version = "0.1.0"
main = "src/lib.loz"
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("src/lib.loz"),
            r#"module text_utils;

func title() -> Text {
    return "Hello from package";
}
"#,
        )
        .unwrap();
        fs::write(
            dir.join("src/main.loz"),
            r#"import text_utils;

func main() -> Text {
    return text_utils.title();
}
"#,
        )
        .unwrap();

        let program = load_program_from_entry(&dir.join("src/main.loz")).unwrap();
        analyze(&program).unwrap();
        let result = execute(&program).unwrap();
        let ir = generate_llvm_ir(&program).unwrap();

        assert_eq!(result, RuntimeValue::Text("Hello from package".to_string()));
        assert!(ir.contains("define"));
        assert!(ir.contains("title"));
    }

    #[test]
    fn import_builtin_namespace_is_accepted() {
        let dir = make_temp_dir("loz_builtin_import");
        let main_path = dir.join("main.loz");
        fs::write(
            &main_path,
            r#"import json;

func main() -> Text {
    return json.stringify(json.parse("{\"name\":\"Ahmed\"}"));
}
"#,
        )
        .unwrap();

        let program = load_program_from_entry(&main_path).unwrap();
        analyze(&program).unwrap();
        let result = execute(&program).unwrap();

        assert_eq!(
            result,
            RuntimeValue::Text("{\"name\":\"Ahmed\"}".to_string())
        );
    }

    #[test]
    fn rejects_unknown_import() {
        let dir = make_temp_dir("loz_unknown_import");
        let main_path = dir.join("main.loz");
        fs::write(
            &main_path,
            r#"import missing;

func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let error = load_program_from_entry(&main_path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("dependency 'missing' not found in loz.toml")
        );
    }

    #[test]
    fn rejects_unknown_imported_function() {
        let dir = make_temp_dir("loz_unknown_imported_function");
        let package_dir = dir.join("packages/text_utils");
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(package_dir.join("src")).unwrap();

        fs::write(
            dir.join("loz.toml"),
            r#"[project]
name = "demo"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("loz.toml"),
            r#"[project]
name = "text_utils"
main = "src/lib.loz"
"#,
        )
        .unwrap();
        fs::write(
            package_dir.join("src/lib.loz"),
            r#"module text_utils;

func title() -> Text {
    return "Hello";
}
"#,
        )
        .unwrap();
        fs::write(
            dir.join("src/main.loz"),
            r#"import text_utils;

func main() -> Text {
    return text_utils.missing();
}
"#,
        )
        .unwrap();

        let error = load_checked_program(&dir.join("src/main.loz")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module 'text_utils' has no function 'missing'")
        );
    }

    #[test]
    fn renders_dependency_list() {
        let dependencies = vec![ResolvedDependency {
            alias: "text_utils".to_string(),
            path_text: "./packages/text_utils".to_string(),
            package_root: PathBuf::from("/tmp/text_utils"),
            main_relative: PathBuf::from("src/lib.loz"),
            main_path: PathBuf::from("/tmp/text_utils/src/lib.loz"),
        }];

        let rendered = render_dependencies(&dependencies);
        assert!(rendered.contains("Dependencies:"));
        assert!(rendered.contains("text_utils"));
        assert!(rendered.contains("path: ./packages/text_utils"));
        assert!(rendered.contains("main: src/lib.loz"));
    }

    #[test]
    fn parses_valid_loz_toml() {
        let dir = make_temp_dir("loz_valid_toml");
        let config_path = dir.join("loz.toml");
        fs::write(
            &config_path,
            r#"[project]
name = "support-agent"
version = "0.1.0"
main = "src/main.loz"

[python]
path = "python3"

[llm]
provider = "ollama"
model = "qwen2.5:0.5b"
"#,
        )
        .unwrap();

        let config = load_project_file_config_from_path(&config_path).unwrap();
        assert_eq!(
            config
                .project
                .as_ref()
                .and_then(|project| project.main.as_deref()),
            Some("src/main.loz")
        );
        assert_eq!(
            config
                .python
                .as_ref()
                .and_then(|python| python.path.as_deref()),
            Some("python3")
        );
        assert_eq!(
            config.llm.as_ref().and_then(|llm| llm.provider.as_deref()),
            Some("ollama")
        );
    }

    #[test]
    fn missing_loz_toml_falls_back_for_explicit_source() {
        let dir = make_temp_dir("loz_missing_toml");
        let source_path = dir.join("main.loz");
        fs::write(
            &source_path,
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let context = resolve_project_context(Some(Path::new("main.loz")), &dir).unwrap();

        assert_eq!(context.project_root, fs::canonicalize(&dir).unwrap());
        assert_eq!(context.source_path, fs::canonicalize(&source_path).unwrap());
        assert!(load_project_file_config(&dir).unwrap().project.is_none());
    }

    #[test]
    fn invalid_loz_toml_returns_clear_error() {
        let dir = make_temp_dir("loz_invalid_toml");
        let config_path = dir.join("loz.toml");
        let source_path = dir.join("main.loz");
        fs::write(&config_path, "[project]\nmain = 123\n").unwrap();
        fs::write(
            &source_path,
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let error = resolve_project_context(Some(&source_path), &dir).unwrap_err();
        assert!(error.to_string().contains("invalid loz.toml"));
    }

    #[test]
    fn dotenv_values_load_and_env_overrides_them() {
        let dir = make_temp_dir("loz_dotenv");
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "main.loz"

[python]
path = "python-from-toml"

[llm]
provider = "mock"
"#,
        )
        .unwrap();
        fs::write(
            dir.join(".env"),
            "LOZ_PYTHON_PATH=python-from-dotenv\nLOZ_LLM_PROVIDER=ollama\n",
        )
        .unwrap();
        fs::write(
            dir.join("main.loz"),
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        with_process_state(
            &dir,
            &[
                ("LOZ_PYTHON_PATH", None),
                ("LOZ_LLM_PROVIDER", Some("github")),
                ("LOZ_GITHUB_TOKEN_ENV", None),
            ],
            || {
                let context = resolve_project_context(None, &dir).unwrap();
                context.apply_runtime_environment().unwrap();

                assert_eq!(
                    std::env::var("LOZ_PYTHON_PATH").unwrap(),
                    "python-from-dotenv"
                );
                assert_eq!(std::env::var("LOZ_LLM_PROVIDER").unwrap(), "github");
            },
        );
    }

    #[test]
    fn omitted_source_uses_project_main() {
        let dir = make_temp_dir("loz_project_main");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "src/main.loz"
"#,
        )
        .unwrap();
        let main_path = src_dir.join("main.loz");
        fs::write(
            &main_path,
            r#"func main() -> Int {
    return 0;
}
"#,
        )
        .unwrap();

        let context = resolve_project_context(None, &dir).unwrap();

        assert_eq!(context.project_root, fs::canonicalize(&dir).unwrap());
        assert_eq!(context.source_path, fs::canonicalize(main_path).unwrap());
    }

    #[test]
    fn run_without_source_executes_project_main_with_mock_llm() {
        let dir = make_temp_dir("loz_project_run");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "src/main.loz"

[llm]
provider = "mock"
"#,
        )
        .unwrap();
        fs::write(
            src_dir.join("main.loz"),
            r#"func main() -> Text {
    return llm.ask("hello");
}
"#,
        )
        .unwrap();

        with_process_state(
            &dir,
            &[
                ("LOZ_LLM_PROVIDER", None),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("LOZ_GITHUB_TOKEN_ENV", None),
            ],
            || {
                let context = resolve_project_context(None, &dir).unwrap();
                context.apply_runtime_environment().unwrap();
                let _working_directory =
                    super::WorkingDirectoryGuard::change_to(&context.project_root).unwrap();

                let program = load_program_from_entry(&context.source_path).unwrap();
                analyze(&program).unwrap();
                let result = execute(&program).unwrap();

                assert_eq!(result, RuntimeValue::Text("[mock] hello".to_string()));
            },
        );
    }

    #[test]
    fn python_interop_respects_project_root() {
        if !python_is_available() {
            return;
        }

        let dir = make_temp_dir("loz_project_python");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "src/main.loz"

[python]
path = "python3"
"#,
        )
        .unwrap();
        fs::write(
            dir.join("tools.py"),
            r#"def analyze_text(payload):
    text = payload["text"]
    return {"length": len(text), "label": "ok"}
"#,
        )
        .unwrap();
        fs::write(
            src_dir.join("main.loz"),
            r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.call("tools.analyze_text", payload);
    return json.get_i32(result, "length");
}
"#,
        )
        .unwrap();

        with_process_state(
            &dir,
            &[
                ("LOZ_PYTHON_PATH", None),
                ("LOZ_GITHUB_TOKEN_ENV", None),
                ("LOZ_LLM_PROVIDER", None),
            ],
            || {
                let context = resolve_project_context(None, &dir).unwrap();
                context.apply_runtime_environment().unwrap();
                let _working_directory =
                    super::WorkingDirectoryGuard::change_to(&context.project_root).unwrap();

                let program = load_program_from_entry(&context.source_path).unwrap();
                analyze(&program).unwrap();
                let result = execute(&program).unwrap();

                assert_eq!(result, RuntimeValue::Int(5));
            },
        );
    }

    #[test]
    fn compiler_workspace_root_is_discoverable() {
        let root = find_compiler_workspace_root().unwrap();
        assert!(root.join("crates/loz_runtime/Cargo.toml").is_file());
    }

    #[test]
    fn parses_program_agent_and_workflow_cli_commands() {
        let program_invocation = super::parse_cli_args(["loz", "run", "main.loz"]).unwrap();
        assert_eq!(
            program_invocation.command,
            CliCommand::Program(ProgramCommandInvocation {
                kind: ProgramCommandKind::Run,
                source_argument: Some(PathBuf::from("main.loz")),
            })
        );

        let agent_invocation =
            super::parse_cli_args(["loz", "agent", "run", "SupportAgent", "answer", "hello"])
                .unwrap();
        assert_eq!(
            agent_invocation.command,
            CliCommand::Agent(AgentCommandInvocation::Run {
                raw_arguments: vec![
                    "SupportAgent".to_string(),
                    "answer".to_string(),
                    "hello".to_string()
                ],
            })
        );

        let workflow_invocation =
            super::parse_cli_args(["loz", "workflow", "run", "Onboarding"]).unwrap();
        assert_eq!(
            workflow_invocation.command,
            CliCommand::Workflow(super::WorkflowCommandInvocation::Run {
                raw_arguments: vec!["Onboarding".to_string()],
            })
        );
    }

    #[test]
    fn lists_agent_metadata() {
        let program = parse_checked_program(
            r#"agent SupportAgent {
    model: "mock";
    tools: [get_user];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#,
        );

        let agents = collect_agents(&program);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "SupportAgent");
        assert_eq!(agents[0].model.as_deref(), Some("mock"));
        assert_eq!(agents[0].tools, vec!["get_user".to_string()]);
        assert_eq!(agents[0].tasks.len(), 1);
        assert_eq!(agents[0].tasks[0].name, "answer");
        assert_eq!(agents[0].tasks[0].parameters.len(), 1);
        assert_eq!(agents[0].tasks[0].parameters[0].name, "question");
        assert_eq!(
            agents[0].tasks[0].parameters[0].type_name,
            loz_ast::TypeName::Text
        );
        assert_eq!(agents[0].tasks[0].return_type, loz_ast::TypeName::Text);

        let rendered = render_agent_list(&agents);
        assert!(rendered.contains("Agents found:"));
        assert!(rendered.contains("SupportAgent"));
        assert!(rendered.contains("model: mock"));
        assert!(rendered.contains("answer(question: Text) -> Text"));
    }

    #[test]
    fn auto_selects_single_agent_and_task() {
        let agents = vec![DiscoveredAgent {
            name: "SupportAgent".to_string(),
            model: Some("mock".to_string()),
            tools: vec![],
            tasks: vec![DiscoveredTask {
                name: "answer".to_string(),
                parameters: vec![loz_ast::FunctionParameter {
                    name: "question".to_string(),
                    type_name: loz_ast::TypeName::Text,
                    span: loz_ast::Span::default(),
                }],
                return_type: loz_ast::TypeName::Text,
            }],
        }];

        let cli_arguments = vec!["hello".to_string()];
        let selection = resolve_agent_task_selection(&agents, &cli_arguments).unwrap();
        assert_eq!(selection.agent.name, "SupportAgent");
        assert_eq!(selection.task.name, "answer");
        assert_eq!(selection.arguments, &["hello".to_string()]);
    }

    #[test]
    fn rejects_ambiguous_shortcut_mode() {
        let agents = vec![
            DiscoveredAgent {
                name: "SupportAgent".to_string(),
                model: Some("mock".to_string()),
                tools: vec![],
                tasks: vec![DiscoveredTask {
                    name: "answer".to_string(),
                    parameters: vec![],
                    return_type: loz_ast::TypeName::Text,
                }],
            },
            DiscoveredAgent {
                name: "SalesAgent".to_string(),
                model: Some("mock".to_string()),
                tools: vec![],
                tasks: vec![DiscoveredTask {
                    name: "pitch".to_string(),
                    parameters: vec![],
                    return_type: loz_ast::TypeName::Text,
                }],
            },
        ];

        let error = ambiguous_agent_selection_error(&agents);
        assert!(
            error
                .to_string()
                .contains("cannot auto-select agent/task because multiple candidates exist")
        );
        assert!(error.to_string().contains("SupportAgent"));
        assert!(error.to_string().contains("SalesAgent"));
    }

    #[test]
    fn parses_supported_agent_argument_types() {
        let text_parameter = loz_ast::FunctionParameter {
            name: "question".to_string(),
            type_name: loz_ast::TypeName::Text,
            span: loz_ast::Span::default(),
        };
        let i32_parameter = loz_ast::FunctionParameter {
            name: "count".to_string(),
            type_name: loz_ast::TypeName::I32,
            span: loz_ast::Span::default(),
        };
        let i64_parameter = loz_ast::FunctionParameter {
            name: "big".to_string(),
            type_name: loz_ast::TypeName::I64,
            span: loz_ast::Span::default(),
        };
        let f64_parameter = loz_ast::FunctionParameter {
            name: "ratio".to_string(),
            type_name: loz_ast::TypeName::F64,
            span: loz_ast::Span::default(),
        };
        let bool_parameter = loz_ast::FunctionParameter {
            name: "enabled".to_string(),
            type_name: loz_ast::TypeName::Bool,
            span: loz_ast::Span::default(),
        };
        let json_parameter = loz_ast::FunctionParameter {
            name: "payload".to_string(),
            type_name: loz_ast::TypeName::Json,
            span: loz_ast::Span::default(),
        };

        assert_eq!(
            parse_task_argument(&text_parameter, "hello").unwrap(),
            RuntimeValue::Text("hello".to_string())
        );
        assert_eq!(
            parse_task_argument(&i32_parameter, "42").unwrap(),
            RuntimeValue::Int(42)
        );
        assert_eq!(
            parse_task_argument(&i64_parameter, "9000").unwrap(),
            RuntimeValue::Int(9000)
        );
        assert_eq!(
            parse_task_argument(&f64_parameter, "3.5").unwrap(),
            RuntimeValue::Float(3.5)
        );
        assert_eq!(
            parse_task_argument(&bool_parameter, "yes").unwrap(),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            parse_task_argument(&json_parameter, "{\"name\":\"Ahmed\"}").unwrap(),
            RuntimeValue::Json(serde_json::json!({"name":"Ahmed"}))
        );
    }

    #[test]
    fn formats_agent_task_results() {
        let task = DiscoveredTask {
            name: "answer".to_string(),
            parameters: vec![],
            return_type: loz_ast::TypeName::Text,
        };

        assert_eq!(
            format_task_result(&task, RuntimeValue::Text("hello".to_string())).unwrap(),
            "hello"
        );
        assert_eq!(
            format_task_result(&task, RuntimeValue::Json(serde_json::json!({"id": 1}))).unwrap(),
            "{\"id\":1}"
        );
    }

    #[test]
    fn reports_unknown_agent_task_and_argument_errors() {
        let agents = vec![DiscoveredAgent {
            name: "SupportAgent".to_string(),
            model: Some("mock".to_string()),
            tools: vec![],
            tasks: vec![DiscoveredTask {
                name: "score".to_string(),
                parameters: vec![loz_ast::FunctionParameter {
                    name: "value".to_string(),
                    type_name: loz_ast::TypeName::I32,
                    span: loz_ast::Span::default(),
                }],
                return_type: loz_ast::TypeName::I32,
            }],
        }];

        let unknown_agent_arguments = vec!["Missing".to_string(), "score".to_string()];
        let unknown_agent =
            resolve_agent_task_selection(&agents, &unknown_agent_arguments).unwrap_err();
        assert!(
            unknown_agent
                .to_string()
                .contains("agent 'Missing' not found")
        );

        let unknown_task_arguments = vec!["SupportAgent".to_string(), "ask".to_string()];
        let unknown_task =
            resolve_agent_task_selection(&agents, &unknown_task_arguments).unwrap_err();
        assert!(
            unknown_task
                .to_string()
                .contains("task 'ask' not found in agent 'SupportAgent'")
        );

        let wrong_count = parse_task_arguments(&agents[0].tasks[0], &[]).unwrap_err();
        assert!(
            wrong_count
                .to_string()
                .contains("expects 1 argument, got 0")
        );

        let wrong_type =
            parse_task_arguments(&agents[0].tasks[0], &["abc".to_string()]).unwrap_err();
        assert!(
            wrong_type
                .to_string()
                .contains("argument 1 for task 'score' must be i32, got 'abc'")
        );
    }

    #[test]
    fn resolves_explicit_agent_run_source_argument() {
        let dir = make_temp_dir("loz_agent_source");
        let source_path = dir.join("main.loz");
        fs::write(&source_path, "func main() -> i32 { return 0; }\n").unwrap();

        let (source_argument, consumed) = split_agent_run_source_argument(
            &[
                "main.loz".to_string(),
                "SupportAgent".to_string(),
                "answer".to_string(),
            ],
            &dir,
        );

        assert_eq!(source_argument, Some(PathBuf::from("main.loz")));
        assert_eq!(consumed, 1);
    }

    #[test]
    fn agent_run_executes_mock_llm_and_json_task() {
        let llm_program = parse_checked_program(
            r#"agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#,
        );
        let llm_agents = collect_agents(&llm_program);

        with_process_state(
            Path::new("/tmp"),
            &[
                ("LOZ_LLM_PROVIDER", Some("mock")),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("LOZ_GITHUB_TOKEN_ENV", None),
            ],
            || {
                let shortcut_arguments = vec!["hello".to_string()];
                let output =
                    run_agent_task(&llm_program, &llm_agents, &shortcut_arguments).unwrap();
                assert_eq!(output, "[mock] hello");
            },
        );

        let json_program = parse_checked_program(
            r#"agent DataAgent {
    model: "mock";
    tools: [];

    task get_name(user: Json) -> Text {
        return json.get_text(user, "name");
    }
}

func main() -> i32 {
    return 0;
}
"#,
        );
        let json_agents = collect_agents(&json_program);
        let output = run_agent_task(
            &json_program,
            &json_agents,
            &[
                "DataAgent".to_string(),
                "get_name".to_string(),
                "{\"name\":\"Ahmed\"}".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(output, "Ahmed");
    }

    #[test]
    fn labels_task_types_for_usage() {
        assert_eq!(type_name_label(&loz_ast::TypeName::Text), "Text");
        assert_eq!(type_name_label(&loz_ast::TypeName::I32), "i32");
        assert_eq!(type_name_label(&loz_ast::TypeName::Json), "Json");
    }

    #[test]
    fn lists_workflow_metadata() {
        let program = parse_checked_program(
            r#"func prepare() -> Text {
    return "prepared";
}

workflow Onboarding {
    step prepare;
}

func main() -> i32 {
    return 0;
}
"#,
        );

        let workflows = collect_workflows(&program);
        assert_eq!(
            workflows,
            vec![DiscoveredWorkflow {
                name: "Onboarding".to_string(),
                steps: vec![DiscoveredWorkflowStep {
                    name: "prepare".to_string(),
                    target: loz_ast::WorkflowTarget::FunctionOrTool("prepare".to_string()),
                }],
            }]
        );

        let rendered = render_workflow_list(&workflows);
        assert!(rendered.contains("Workflows found:"));
        assert!(rendered.contains("Onboarding"));
        assert!(rendered.contains("1. prepare"));
    }

    #[test]
    fn resolves_workflow_shortcut_and_reports_errors() {
        let workflows = vec![DiscoveredWorkflow {
            name: "Onboarding".to_string(),
            steps: vec![DiscoveredWorkflowStep {
                name: "prepare".to_string(),
                target: loz_ast::WorkflowTarget::FunctionOrTool("prepare".to_string()),
            }],
        }];

        let selected = resolve_workflow_selection(&workflows, None).unwrap();
        assert_eq!(selected.name, "Onboarding");

        let missing = resolve_workflow_selection(&workflows, Some("Missing")).unwrap_err();
        assert!(missing.to_string().contains("workflow 'Missing' not found"));

        let ambiguous = resolve_workflow_selection(
            &[
                workflows[0].clone(),
                DiscoveredWorkflow {
                    name: "BillingFlow".to_string(),
                    steps: vec![DiscoveredWorkflowStep {
                        name: "done".to_string(),
                        target: loz_ast::WorkflowTarget::FunctionOrTool("done".to_string()),
                    }],
                },
            ],
            None,
        )
        .unwrap_err();
        assert!(
            ambiguous
                .to_string()
                .contains("cannot auto-select workflow because multiple workflows exist")
        );
    }

    #[test]
    fn resolves_explicit_workflow_run_source_argument() {
        let dir = make_temp_dir("loz_workflow_source");
        let source_path = dir.join("main.loz");
        fs::write(&source_path, "func main() -> i32 { return 0; }\n").unwrap();

        let (source_argument, consumed) = split_workflow_run_source_argument(
            &["main.loz".to_string(), "Onboarding".to_string()],
            &dir,
        );

        assert_eq!(source_argument, Some(PathBuf::from("main.loz")));
        assert_eq!(consumed, 1);
    }

    #[test]
    fn workflow_run_executes_and_formats_step_results() {
        let program = parse_checked_program(
            r#"func prepare() -> Text {
    return "prepared";
}

tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

workflow Onboarding {
    step prepare;
    step get_data;
}

func main() -> i32 {
    return 0;
}
"#,
        );

        let workflows = collect_workflows(&program);
        let output = run_workflow(&program, &workflows, Some("Onboarding")).unwrap();
        assert!(output.contains("[1/2] prepare"));
        assert!(output.contains("prepared"));
        assert!(output.contains("[2/2] get_data"));
        assert!(output.contains("{\"name\":\"Ahmed\"}"));
    }

    #[test]
    fn parses_check_command() {
        let invocation = super::parse_cli_args(["loz", "check", "main.loz"]).unwrap();
        assert_eq!(
            invocation.command,
            CliCommand::Program(ProgramCommandInvocation {
                kind: ProgramCommandKind::Check,
                source_argument: Some(PathBuf::from("main.loz")),
            })
        );
    }

    #[test]
    fn parses_doctor_command() {
        let invocation = super::parse_cli_args(["loz", "doctor"]).unwrap();
        assert_eq!(invocation.command, CliCommand::Doctor);
    }

    #[test]
    fn parses_init_command() {
        let invocation = super::parse_cli_args(["loz", "init", "support-agent"]).unwrap();
        assert_eq!(
            invocation.command,
            CliCommand::Init {
                project_path: PathBuf::from("support-agent"),
            }
        );
    }

    #[test]
    fn parses_help_and_version_flags() {
        assert_eq!(
            super::parse_cli_args(["loz", "--help"]).unwrap().command,
            CliCommand::Help
        );
        assert_eq!(
            super::parse_cli_args(["loz", "-h"]).unwrap().command,
            CliCommand::Help
        );
        assert_eq!(
            super::parse_cli_args(["loz", "--version"]).unwrap().command,
            CliCommand::Version
        );
        assert_eq!(
            super::parse_cli_args(["loz", "-V"]).unwrap().command,
            CliCommand::Version
        );
    }

    #[test]
    fn check_loads_valid_program() {
        let dir = make_temp_dir("loz_check_valid");
        let source_path = dir.join("main.loz");
        fs::write(
            &source_path,
            r#"func main() -> i32 {
    const value: i64 = 40 + 2;
    print(value);
    return 0;
}
"#,
        )
        .unwrap();

        let program = load_checked_program(&source_path).unwrap();
        assert!(!program.statements.is_empty());
    }

    #[test]
    fn init_creates_expected_project_files() {
        let dir = make_temp_dir("loz_init_parent");
        let target = dir.join("support-agent");

        run_init(&dir, Path::new("support-agent")).unwrap();

        assert!(target.join("loz.toml").is_file());
        assert!(target.join(".env.example").is_file());
        assert!(target.join("src/main.loz").is_file());
        assert!(target.join("tools/tools.py").is_file());
        assert!(target.join("examples/hello.loz").is_file());
        assert!(target.join("README.md").is_file());
        assert!(
            fs::read_to_string(target.join("src/main.loz"))
                .unwrap()
                .contains("workflow DemoFlow")
        );
    }

    #[test]
    fn init_refuses_non_empty_directory() {
        let dir = make_temp_dir("loz_init_existing");
        let target = dir.join("support-agent");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("existing.txt"), "keep").unwrap();

        let error = run_init(&dir, Path::new("support-agent")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("directory already exists and is not empty")
        );
    }

    #[test]
    fn doctor_report_contains_key_sections() {
        let dir = make_temp_dir("loz_doctor");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "src/main.loz"

[llm]
provider = "mock"
"#,
        )
        .unwrap();
        fs::write(
            src_dir.join("main.loz"),
            "func main() -> i32 { return 0; }\n",
        )
        .unwrap();

        let report = with_process_state(
            &dir,
            &[("LOZ_LLM_PROVIDER", Some("mock")), ("GITHUB_TOKEN", None)],
            || collect_doctor_report(&dir).unwrap(),
        );
        let rendered = render_doctor_report(&report);

        assert!(rendered.contains("Loz doctor"));
        assert!(rendered.contains("Platform:"));
        assert!(rendered.contains("Project:"));
        assert!(rendered.contains("Toolchain:"));
        assert!(rendered.contains("Runtime:"));
        assert!(rendered.contains("Status:"));
    }

    #[test]
    fn doctor_reports_interpreter_ready_when_native_tools_are_missing() {
        let dir = make_temp_dir("loz_doctor_interpreter_only");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(dir.join("loz.toml"), "[project]\nmain = \"src/main.loz\"\n").unwrap();
        fs::write(
            src_dir.join("main.loz"),
            "func main() -> i32 { return 0; }\n",
        )
        .unwrap();

        let report = with_process_state(
            &dir,
            &[
                ("PATH", Some("")),
                ("LOZ_CLANG_PATH", None),
                ("LOZ_LLC_PATH", None),
                ("LOZ_CARGO_PATH", None),
                ("LOZ_PYTHON_PATH", None),
            ],
            || collect_doctor_report(&dir).unwrap(),
        );
        let rendered = render_doctor_report(&report);

        assert!(rendered.contains("os: ok -"));
        assert!(rendered.contains("cargo: warning -"));
        assert!(rendered.contains("clang: warning -"));
        assert!(rendered.contains("llc: warning -"));
        assert!(rendered.contains("ready for interpreter, native build unavailable"));
    }

    #[test]
    fn doctor_marks_missing_main_as_error() {
        let dir = make_temp_dir("loz_doctor_missing_main");
        fs::write(
            dir.join("loz.toml"),
            r#"[project]
main = "src/missing.loz"
"#,
        )
        .unwrap();

        let report = collect_doctor_report(&dir).unwrap();
        let main_item = report
            .sections
            .iter()
            .find(|section| section.title == "Project")
            .unwrap()
            .items
            .iter()
            .find(|item| item.label == "main")
            .unwrap();
        assert_eq!(main_item.severity, DoctorSeverity::Error);
    }

    #[test]
    fn load_checked_program_renders_semantic_diagnostic() {
        let dir = make_temp_dir("loz_check_diag");
        let source_path = dir.join("bad.loz");
        let source = "func main() -> i32 {\n    print(username);\n    return 0;\n}\n";
        fs::write(&source_path, source).unwrap();

        let error = load_checked_program(&source_path).unwrap_err();
        let rendered = error.to_string();

        assert!(rendered.contains("bad.loz:2:11"));
        assert!(rendered.contains("unknown identifier 'username'"));
        assert!(rendered.contains("print(username);"));
        assert!(rendered.contains("^"));
    }

    #[test]
    fn project_mode_check_reports_main_file_location() {
        let dir = make_temp_dir("loz_project_diag");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(dir.join("loz.toml"), "[project]\nmain = \"src/main.loz\"\n").unwrap();
        fs::write(
            src_dir.join("main.loz"),
            "func main() -> i32 {\n    print(\"hello\")\n    return 0;\n}\n",
        )
        .unwrap();

        let context = resolve_project_context(None, &dir).unwrap();
        let error = load_checked_program(&context.source_path).unwrap_err();
        let rendered = error.to_string();

        assert!(rendered.contains("main.loz:2:19"));
        assert!(rendered.contains("expected ';' after expression"));
    }

    #[test]
    fn infers_project_name_from_target_path() {
        let path = Path::new("/tmp/support-agent");
        assert_eq!(infer_project_name(path).unwrap(), "support-agent");
        assert!(init_main_source().contains("Loz project ready"));
    }
}
