//! MCP tool surface for the FMECA kernel.
//!
//! [`FmecaServer`] wraps an [`Engine`] and exposes the command/query tools plus
//! the stateless `analyze` batch tool over MCP. The server is **thin**: every
//! handler parses args, calls one
//! `Engine` method, and serializes the result. All structure/state lives in the
//! kernel (`fmeca`).
//!
//! # Tool surface
//!
//! | Tool               | Kind    | Engine method                  |
//! |--------------------|---------|--------------------------------|
//! | `session.open`     | command | [`Engine::open_session_with_meta`] |
//! | `append`           | command | `add_failure_mode`/`add_mitigation`/`rescore`/`set_mitigation_status`/`set_failure_mode_disposition` |
//! | `state.get`        | query   | [`Engine::state`]              |
//! | `risk.next`        | query   | [`Engine::risk_next`]          |
//! | `readiness.assess` | query   | [`Engine::readiness`]          |
//! | `report.export`    | query   | [`Engine::export`]             |
//! | `scoring.catalog`  | query   | [`fmeca::catalog`]        |
//! | `analyze`          | batch   | [`fmeca::analyze()`]      |
//! | `session.list`     | query   | [`Engine::list_sessions`] + [`Engine::state`] |
//! | `actions.suggest`  | query   | `fmeca_policy::evaluate` |
//! | `policy.list`      | query   | `fmeca_policy::preset` + project files |
//! | `vet`              | advisory | Jev second opinion (feature `jev`) |
//!
//! Errors propagate the kernel's stable prefixes (`SESSION_NOT_FOUND`,
//! `INVALID_FAILURE_MODE`, `BAD_SESSION_ID`, …) verbatim in the MCP error
//! message.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use fmeca::{
    AnalysisLevel, AnalyzeInput, Disposition, DispositionChange, Engine, FailureMode,
    FailureModeStanding, FmecaError, FmecaState, MatrixStrategy, Mitigation, MitigationStatus,
    MitigationStatusChange, Rescore, ResolutionEvidence, ScoreCriterion, SessionMeta,
};
use rmcp::ErrorData as McpError;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
    InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "jev")]
use crate::jev::{JudgmentModel, Unavailable};

/// Tool names, `noun.verb` like the rest of the workspace.
pub const TOOL_SESSION_OPEN: &str = "session.open";
pub const TOOL_APPEND: &str = "append";
pub const TOOL_STATE_GET: &str = "state.get";
pub const TOOL_RISK_NEXT: &str = "risk.next";
pub const TOOL_READINESS_ASSESS: &str = "readiness.assess";
pub const TOOL_REPORT_EXPORT: &str = "report.export";
pub const TOOL_SCORING_CATALOG: &str = "scoring.catalog";
pub const TOOL_ANALYZE: &str = "analyze";
/// Lists the analyses in the state dir with their metadata and gates.
pub const TOOL_SESSION_LIST: &str = "session.list";
/// Suggests actions for an analysis under a declarative risk policy (read-only).
pub const TOOL_ACTIONS_SUGGEST: &str = "actions.suggest";
/// Lists the built-in and project risk policies (read-only).
pub const TOOL_POLICY_LIST: &str = "policy.list";
/// The advisory Jev tool (feature `jev`): probabilities beside the
/// deterministic results, never instead of them.
#[cfg(feature = "jev")]
pub const TOOL_VET: &str = "vet";

/// All tool names in declaration order. The six core command/query tools, the
/// `scoring.catalog` query (the fixed observation vocabulary the caller draws
/// from), the stateless one-shot `analyze` batch tool and, with the `jev`
/// feature, the advisory `vet` tool.
pub const TOOL_NAMES: &[&str] = &[
    TOOL_SESSION_OPEN,
    TOOL_APPEND,
    TOOL_STATE_GET,
    TOOL_RISK_NEXT,
    TOOL_READINESS_ASSESS,
    TOOL_REPORT_EXPORT,
    TOOL_SCORING_CATALOG,
    TOOL_ANALYZE,
    TOOL_SESSION_LIST,
    TOOL_ACTIONS_SUGGEST,
    TOOL_POLICY_LIST,
    #[cfg(feature = "jev")]
    TOOL_VET,
];

// --- wire arg structs ------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionIdArgs {
    session_id: String,
}

/// `session.open` args: a session id plus an OPTIONAL, closed
/// `matrix_strategy` selection. Defaults to the 3×3 strategy (back-compat). The
/// caller SELECTS a strategy; it cannot edit cells. The analysis metadata
/// (spec §6.2) is optional too and, like the strategy, fixed once opened.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionOpenArgs {
    session_id: String,
    #[serde(default)]
    matrix_strategy: MatrixStrategy,
    title: Option<String>,
    level: Option<AnalysisLevel>,
    aspect: Option<String>,
    #[serde(default)]
    scope_paths: Vec<String>,
    parent: Option<String>,
}

/// `session.list` args: optional filters, each an exact match on the
/// session's metadata.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionListArgs {
    level: Option<AnalysisLevel>,
    aspect: Option<String>,
    parent: Option<String>,
}

/// The `append` command is a tagged union over its three variants.
#[derive(Debug, Deserialize)]
#[serde(tag = "variant", rename_all = "snake_case", deny_unknown_fields)]
enum AppendArgs {
    AddFailureMode {
        session_id: String,
        // Boxed to balance the enum's variant sizes (clippy `large_enum_variant`).
        failure_mode: Box<FailureMode>,
    },
    AddMitigation {
        session_id: String,
        mitigation: Mitigation,
    },
    Rescore {
        session_id: String,
        rescore: Rescore,
    },
    /// Move a mitigation through planned → implemented → verified (or
    /// abandoned). The server stamps `at`.
    SetMitigationStatus {
        session_id: String,
        mitigation_id: String,
        status: MitigationStatus,
        #[serde(default)]
        evidence: Vec<ResolutionEvidence>,
        note: Option<String>,
    },
    /// Accept a failure mode's residual risk, exclude it as not credible, or
    /// reopen it. The server stamps `at`.
    SetFailureModeDisposition {
        session_id: String,
        failure_mode_id: String,
        disposition: Disposition,
        rationale: String,
        approver: Option<String>,
        #[serde(default)]
        evidence: Vec<ResolutionEvidence>,
    },
}

/// `risk.next` response wrapper: the highest-criticality unmitigated
/// failure mode, or `null`.
#[derive(Debug, Serialize)]
struct RiskNextResponse {
    risk: Option<fmeca::FailureModeView>,
}

/// `scoring.catalog` args: an OPTIONAL strategy selection (default 3×3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoringCatalogArgs {
    #[serde(default)]
    matrix_strategy: MatrixStrategy,
}

/// `scoring.catalog` response: the selected strategy + its
/// fixed evidence→score criteria.
#[derive(Debug, Serialize)]
struct ScoringCatalogResponse {
    matrix_strategy: MatrixStrategy,
    criteria: Vec<ScoreCriterion>,
}

/// `analyze` args: a stateless, one-shot FMECA over an ENTIRE batch of failure
/// modes. Optional `matrix_strategy` SELECTS the criticality matrix (default
/// 3×3). No session, no persistence — `(input) → (computed report)`, reusing the
/// EXACT same kernel compute as the session path.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalyzeArgs {
    #[serde(default)]
    matrix_strategy: MatrixStrategy,
    failure_modes: Vec<AnalyzeInput>,
}

// --- server ----------------------------------------------------------------

/// MCP server façade over an [`Engine`]. Cheap to clone.
#[derive(Clone)]
pub struct FmecaServer {
    engine: Engine,
    /// The directory the engine's store lives in, reported by `session.list`.
    state_dir: PathBuf,
    server_name: String,
    server_version: String,
    /// The Jev judge `vet` asks; `None` when no key is configured.
    #[cfg(feature = "jev")]
    judge: Option<Arc<dyn JudgmentModel>>,
    /// Why the judge could not be built (a rejected configuration), reported
    /// by `vet` instead of `no_key`.
    #[cfg(feature = "jev")]
    judge_unavailable: Option<Unavailable>,
}

impl FmecaServer {
    /// Build a server backed by the supplied engine.
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            state_dir: crate::resolve_state_dir(),
            server_name: "fmeca-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            #[cfg(feature = "jev")]
            judge: None,
            #[cfg(feature = "jev")]
            judge_unavailable: None,
        }
    }

    /// Set the judge `vet` asks (`None`: `vet` reports `vet_unavailable`).
    #[cfg(feature = "jev")]
    pub fn with_judge(mut self, judge: Option<Arc<dyn JudgmentModel>>) -> Self {
        self.judge = judge;
        self
    }

    /// Record why no judge could be built, so `vet` reports that reason
    /// (e.g. `bad_config`) rather than `no_key`. Ignored while a judge is set.
    #[cfg(feature = "jev")]
    pub fn with_judge_unavailable(mut self, unavailable: Option<Unavailable>) -> Self {
        self.judge_unavailable = unavailable;
        self
    }

    /// Set the state dir reported by `session.list` (`main` passes the
    /// resolved one; tests pass their tempdir).
    pub fn with_state_dir(mut self, state_dir: PathBuf) -> Self {
        self.state_dir = state_dir;
        self
    }

    /// Borrow the inner engine (tests drive state directly).
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Serve over stdio. Blocks until the peer disconnects.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let service = self.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }

    /// The transport-free path of `call_tool`: `vet` (async) is routed
    /// here, every other tool to [`Self::dispatch_call`].
    pub async fn call_tool_value(&self, request: CallToolRequestParams) -> Result<Value, McpError> {
        #[cfg(feature = "jev")]
        if request.name == TOOL_VET {
            return self.vet(request_args(&request)).await;
        }
        self.dispatch_call(request)
    }

    /// The advisory `vet` tool: plan questions about a session or a one-shot
    /// analysis, ask Jev, and report probabilities beside the deterministic
    /// readiness gate. Never writes. A Jev failure is an `Ok` report with
    /// `status: "vet_unavailable"`; only bad arguments are MCP errors.
    #[cfg(feature = "jev")]
    pub async fn vet(&self, args: Value) -> Result<Value, McpError> {
        use crate::vet::{self as v, VetArgs};

        let parsed: VetArgs = parse_args(args)?;
        let max_questions = parsed.max_questions.unwrap_or(v::DEFAULT_MAX_QUESTIONS);
        if max_questions > v::HARD_MAX_QUESTIONS {
            return Err(McpError::invalid_params(
                format!(
                    "max_questions {max_questions} exceeds the hard cap of {}",
                    v::HARD_MAX_QUESTIONS
                ),
                None,
            ));
        }
        let checks: Vec<&str> = match &parsed.checks {
            None => v::CHECKS.to_vec(),
            Some(asked) => {
                if asked.is_empty() {
                    return Err(McpError::invalid_params(
                        "checks must name at least one check",
                        None,
                    ));
                }
                if let Some(bad) = asked.iter().find(|c| !v::CHECKS.contains(&c.as_str())) {
                    return Err(McpError::invalid_params(
                        format!("unknown check '{bad}'; known: {}", v::CHECKS.join(", ")),
                        None,
                    ));
                }
                v::CHECKS
                    .into_iter()
                    .filter(|c| asked.iter().any(|a| a == c))
                    .collect()
            }
        };
        let subject = match (parsed.session_id, parsed.analysis) {
            (Some(session_id), None) => {
                let engine = self.engine.clone();
                let state = tokio::task::spawn_blocking(move || engine.state(&session_id))
                    .await
                    .map_err(|e| McpError::internal_error(format!("vet read failed: {e}"), None))?
                    .map_err(engine_error_to_mcp)?;
                v::subject_from_state(&state)
            }
            (None, Some(analysis)) => {
                v::subject_from_analysis(analysis.matrix_strategy, &analysis.failure_modes)
                    .map_err(engine_error_to_mcp)?
            }
            _ => {
                return Err(McpError::invalid_params(
                    "pass exactly one of session_id and analysis",
                    None,
                ));
            }
        };
        let judge = match &self.judge {
            Some(judge) => Ok(judge.as_ref()),
            None => Err(self
                .judge_unavailable
                .clone()
                .unwrap_or_else(Unavailable::no_key)),
        };
        Ok(v::run_vet_with(judge, subject, &checks, max_questions).await)
    }

    /// Transport-free dispatch entry point (tests call this directly).
    pub fn dispatch_call(&self, request: CallToolRequestParams) -> Result<Value, McpError> {
        let args = request_args(&request);

        match request.name.as_ref() {
            TOOL_SESSION_OPEN => self.handle_session_open(args),
            TOOL_APPEND => self.handle_append(args),
            TOOL_STATE_GET => self.handle_state_get(args),
            TOOL_RISK_NEXT => self.handle_risk_next(args),
            TOOL_READINESS_ASSESS => self.handle_readiness(args),
            TOOL_REPORT_EXPORT => self.handle_export(args),
            TOOL_SCORING_CATALOG => self.handle_scoring_catalog(args),
            TOOL_ANALYZE => self.handle_analyze(args),
            TOOL_SESSION_LIST => self.handle_session_list(args),
            TOOL_ACTIONS_SUGGEST => {
                crate::suggest::actions_suggest(&self.engine, &self.state_dir, args)
            }
            TOOL_POLICY_LIST => crate::suggest::policy_list(&self.state_dir, args),
            #[cfg(feature = "jev")]
            TOOL_VET => Err(McpError::invalid_params(
                "'vet' is async: call it through call_tool (or call_tool_value)",
                None,
            )),
            other => Err(McpError::invalid_params(
                format!(
                    "Unknown tool '{other}'. Available: {}.",
                    TOOL_NAMES.join(", ")
                ),
                None,
            )),
        }
    }

    fn handle_session_open(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionOpenArgs = parse_args(args)?;
        let meta = SessionMeta {
            title: parsed.title,
            level: parsed.level,
            aspect: parsed.aspect,
            scope_paths: parsed.scope_paths,
            parent: parsed.parent,
        };
        let state = self
            .engine
            .open_session_with_meta(&parsed.session_id, parsed.matrix_strategy, meta)
            .map_err(engine_error_to_mcp)?;
        to_value(&state)
    }

    /// `session.list`: one row per analysis in the state dir, sorted by
    /// session id, filtered on the metadata. A log that fails to replay is a
    /// row `{session_id, error}` rather than a failed call; such rows are kept
    /// whatever the filters, since their metadata cannot be read.
    fn handle_session_list(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionListArgs = parse_args(args)?;
        let ids = self.engine.list_sessions().map_err(engine_error_to_mcp)?;
        let mut sessions = Vec::with_capacity(ids.len());
        for id in ids {
            match self.engine.state(&id) {
                Ok(state) => {
                    let meta = &state.meta;
                    let keep = parsed.level.is_none_or(|l| meta.level == Some(l))
                        && parsed
                            .aspect
                            .as_ref()
                            .is_none_or(|a| meta.aspect.as_ref() == Some(a))
                        && parsed
                            .parent
                            .as_ref()
                            .is_none_or(|p| meta.parent.as_ref() == Some(p));
                    if keep {
                        sessions.push(session_row(&state));
                    }
                }
                Err(error) => sessions.push(json!({
                    "session_id": id,
                    "error": error.to_string(),
                })),
            }
        }
        Ok(json!({
            "state_dir": self.state_dir.display().to_string(),
            "sessions": sessions,
        }))
    }

    fn handle_append(&self, args: Value) -> Result<Value, McpError> {
        let parsed: AppendArgs = parse_args(args)?;
        let state = match parsed {
            AppendArgs::AddFailureMode {
                session_id,
                failure_mode,
            } => self.engine.add_failure_mode(&session_id, *failure_mode),
            AppendArgs::AddMitigation {
                session_id,
                mitigation,
            } => self.engine.add_mitigation(&session_id, mitigation),
            AppendArgs::Rescore {
                session_id,
                rescore,
            } => self.engine.rescore(&session_id, rescore),
            AppendArgs::SetMitigationStatus {
                session_id,
                mitigation_id,
                status,
                evidence,
                note,
            } => {
                let change = MitigationStatusChange {
                    session_id: session_id.clone(),
                    mitigation_id,
                    status,
                    evidence,
                    note,
                    at: Some(now_rfc3339()?),
                };
                self.engine.set_mitigation_status(&session_id, change)
            }
            AppendArgs::SetFailureModeDisposition {
                session_id,
                failure_mode_id,
                disposition,
                rationale,
                approver,
                evidence,
            } => {
                let change = DispositionChange {
                    session_id: session_id.clone(),
                    failure_mode_id,
                    disposition,
                    rationale,
                    approver,
                    evidence,
                    at: Some(now_rfc3339()?),
                };
                self.engine
                    .set_failure_mode_disposition(&session_id, change)
            }
        }
        .map_err(engine_error_to_mcp)?;
        to_value(&state)
    }

    fn handle_state_get(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionIdArgs = parse_args(args)?;
        let state = self
            .engine
            .state(&parsed.session_id)
            .map_err(engine_error_to_mcp)?;
        to_value(&state)
    }

    fn handle_risk_next(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionIdArgs = parse_args(args)?;
        let risk = self
            .engine
            .risk_next(&parsed.session_id)
            .map_err(engine_error_to_mcp)?;
        to_value(&RiskNextResponse { risk })
    }

    fn handle_readiness(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionIdArgs = parse_args(args)?;
        let report = self
            .engine
            .readiness(&parsed.session_id)
            .map_err(engine_error_to_mcp)?;
        to_value(&report)
    }

    fn handle_export(&self, args: Value) -> Result<Value, McpError> {
        let parsed: SessionIdArgs = parse_args(args)?;
        let doc = self
            .engine
            .export(&parsed.session_id)
            .map_err(engine_error_to_mcp)?;
        to_value(&doc)
    }

    /// `scoring.catalog`: the fixed observation vocabulary for
    /// a strategy. Accepts an OPTIONAL `matrix_strategy` (default 3×3) so a caller
    /// can inspect any strategy's catalog before opening a session; a session's
    /// ACTIVE catalog is also embedded in `session.open` / `state.get`. The model
    /// supplies observations from this catalog; it never supplies a score.
    fn handle_scoring_catalog(&self, args: Value) -> Result<Value, McpError> {
        let parsed: ScoringCatalogArgs = parse_args(args)?;
        to_value(&ScoringCatalogResponse {
            matrix_strategy: parsed.matrix_strategy,
            criteria: fmeca::catalog_for(parsed.matrix_strategy),
        })
    }

    /// `analyze` (the stateless batch tool): a pure, one-shot FMECA. Parses the
    /// entire batch, runs [`fmeca::analyze`] (no session, no persistence —
    /// the SAME kernel compute as the session path), and serializes the report.
    /// An unknown observation id maps to `INVALID_OBSERVATION` (`invalid_params`).
    fn handle_analyze(&self, args: Value) -> Result<Value, McpError> {
        let parsed: AnalyzeArgs = parse_args(args)?;
        let report = fmeca::analyze(parsed.matrix_strategy, &parsed.failure_modes)
            .map_err(engine_error_to_mcp)?;
        to_value(&report)
    }
}

// --- ServerHandler ---------------------------------------------------------

impl ServerHandler for FmecaServer {
    fn get_info(&self) -> ServerConfig {
        let mut server_info =
            Implementation::new(self.server_name.clone(), self.server_version.clone());
        server_info.title = Some("fmeca-mcp".to_string());
        server_info.description =
            Some("Deterministic, offline structured-FMECA engine over MCP.".to_string());

        let mut info = InitializeResult::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info.instructions = Some(instructions());
        info
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(tool_definitions()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.call_tool_value(request)
            .await
            .map(|value| CallToolResult::structured(value).into())
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_definitions().into_iter().find(|t| t.name == name)
    }

    async fn on_initialized(&self, _context: NotificationContext<RoleServer>) {
        tracing::info!("fmeca-mcp client initialized");
    }
}

// --- helpers ---------------------------------------------------------------

/// The current UTC time in RFC 3339, the `at` stamp on resolution changes.
/// The kernel never reads a clock; the server does it here.
fn now_rfc3339() -> Result<String, McpError> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|e| McpError::internal_error(format!("clock formatting failed: {e}"), None))
}

/// One `session.list` row for a session that replayed.
///
/// Excluded failure modes (not credible) are left out of `failure_modes` and
/// `by_standing`, as they are left out of the gates; `excluded` counts them.
fn session_row(state: &FmecaState) -> Value {
    let (mut unmitigated, mut under_mitigated, mut acceptable) = (0usize, 0usize, 0usize);
    let mut excluded = 0usize;
    for fmv in &state.failure_modes {
        if fmv.disposition == Disposition::Excluded {
            excluded += 1;
            continue;
        }
        match fmv.standing {
            Some(FailureModeStanding::Unmitigated) => unmitigated += 1,
            Some(FailureModeStanding::UnderMitigated) => under_mitigated += 1,
            Some(FailureModeStanding::Acceptable) => acceptable += 1,
            None => {}
        }
    }
    let meta = &state.meta;
    let readiness = &state.readiness;
    json!({
        "session_id": state.session_id,
        "title": meta.title,
        "level": meta.level,
        "aspect": meta.aspect,
        "scope_paths": meta.scope_paths,
        "parent": meta.parent,
        "failure_modes": state.failure_modes.len() - excluded,
        "by_standing": {
            "unmitigated": unmitigated,
            "under_mitigated": under_mitigated,
            "acceptable": acceptable,
        },
        "excluded": excluded,
        "ready": readiness.ready,
        "ready_verified": readiness.verified.ready,
        "resolution": readiness.resolution,
        "critical_items": readiness.critical_items.len(),
        "accepted_risks": readiness.accepted_risks.len(),
    })
}

fn request_args(request: &CallToolRequestParams) -> Value {
    request
        .arguments
        .as_ref()
        .map(|m| Value::Object(m.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, McpError> {
    serde_json::from_value(args)
        .map_err(|e| McpError::invalid_params(format!("invalid arguments: {e}"), None))
}

pub(crate) fn to_value<T: Serialize>(value: &T) -> Result<Value, McpError> {
    serde_json::to_value(value)
        .map_err(|e| McpError::internal_error(format!("response serialisation failed: {e}"), None))
}

/// Map a kernel error to an MCP error, preserving the stable prefix in the
/// message. `BAD_SESSION_ID` / `INVALID_*` / `DUPLICATE_ID` are caller bugs
/// (`invalid_params`); the rest are surfaced as `internal_error`.
pub(crate) fn engine_error_to_mcp(err: FmecaError) -> McpError {
    let msg = err.to_string();
    match err {
        FmecaError::BadSessionId(_)
        | FmecaError::InvalidFailureMode(_)
        | FmecaError::InvalidMitigation(_)
        | FmecaError::InvalidRescore(_)
        | FmecaError::InvalidObservation(_)
        | FmecaError::DuplicateId(_) => McpError::invalid_params(msg, None),
        FmecaError::SessionNotFound(_)
        | FmecaError::FailureModeNotFound(_)
        | FmecaError::StoreError(_) => McpError::internal_error(msg, None),
    }
}

pub(crate) fn schema_object(value: Value) -> Arc<rmcp::model::JsonObject> {
    debug_assert!(value.is_object(), "schema_object expects an object literal");
    let obj = match value.as_object() {
        Some(o) => o.clone(),
        None => serde_json::Map::new(),
    };
    Arc::new(obj)
}

/// Build the advertised tool definitions. Schemas are intentionally lenient
/// on the deep failure-mode/mitigation/rescore shapes (kernel validation is
/// authoritative and returns stable error prefixes); they document the required
/// envelope keys so an agent knows the call shape.
pub fn tool_definitions() -> Vec<Tool> {
    vec![
        Tool::new(
            Cow::Borrowed(TOOL_SESSION_OPEN),
            Cow::Borrowed(
                "Start or resume a session by session_id. Optional matrix_strategy SELECTS the \
                 criticality matrix (closed set: qualitative3x3 (default) | nasa8004_5x5); a \
                 session's strategy is fixed once opened, and you SELECT it — you cannot edit its \
                 cells. Optional analysis metadata, also fixed once opened: title, level \
                 (concept|conops|technical_plan|project_plan|implementation|verification), \
                 aspect, scope_paths[] and parent (an existing session id). Convention: \
                 session_id = <aspect>-<level>. Returns the current FmecaState including meta, \
                 matrix_strategy, matrix_scale, the active scoring_catalog, the component \
                 registry, computed criticality, signals, and readiness.",
            ),
            session_open_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_APPEND),
            Cow::Borrowed(
                "Append one event. Tagged by `variant`: \
                 add_failure_mode{session_id, failure_mode} | \
                 add_mitigation{session_id, mitigation} | \
                 rescore{session_id, rescore} | \
                 set_mitigation_status{session_id, mitigation_id, status \
                 (planned|implemented|verified|abandoned), evidence?[]{kind, reference, basis?}, \
                 note?} (verified needs evidence whose kind is not note) | \
                 set_failure_mode_disposition{session_id, failure_mode_id, disposition \
                 (open|accepted|excluded), rationale, approver?, evidence?[]} (accepted needs an \
                 approver; excluded needs evidence whose kind is not note). The server stamps \
                 `at` on status and disposition changes. Never delete a failure mode: exclude it. \
                 A failure_mode / rescore supplies severity_observations + \
                 probability_observations, and a mitigation supplies \
                 residual_severity_observations + residual_probability_observations \
                 (all ids from scoring.catalog) — NOT a score; the engine derives every level. \
                 Returns the recomputed FmecaState with freshly-computed criticality + signals.",
            ),
            append_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_STATE_GET),
            Cow::Borrowed(
                "Read-only FmecaState projection: failure modes with computed \
                 criticality/residual/standing + mitigations + issues + signals + \
                 registry + readiness.",
            ),
            session_id_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_RISK_NEXT),
            Cow::Borrowed(
                "The highest-criticality UNMITIGATED failure mode to address next. \
                 Returns {risk: null} when nothing unmitigated is High/Medium.",
            ),
            session_id_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_READINESS_ASSESS),
            Cow::Borrowed(
                "Compute the ReadinessReport: ready bool + residual-criticality buckets + \
                 blockers. Ready iff every failure mode is scored, no residual High/Medium \
                 remains, and no weak_mitigation_order issue stands.",
            ),
            session_id_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_REPORT_EXPORT),
            Cow::Borrowed(
                "Emit the FMECA report: a row per failure mode (component, domain, cause, \
                 effect, criticality, residual, standing, response_class, mitigations), \
                 residual buckets, an explicit accepted-risks section, and the verbatim blockers.",
            ),
            session_id_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_SCORING_CATALOG),
            Cow::Borrowed(
                "The fixed evidence→score catalog for a strategy: criteria{id, axis \
                 (severity|probability), level_ordinal, level (label), description} + the \
                 selected matrix_strategy. Optional matrix_strategy arg (default qualitative3x3). \
                 Supply these ids as severity_observations / probability_observations on append — \
                 the engine maps observations to a strategy-relative level (MAX-combine) and the \
                 matrix collapses it to criticality (low|medium|high). The model never supplies a \
                 score. A session's ACTIVE catalog is also embedded in session.open / state.get.",
            ),
            scoring_catalog_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_ANALYZE),
            Cow::Borrowed(
                "Stateless one-shot FMECA: hand it an ENTIRE analysis in one call, get back the \
                 ENTIRE computed report. NO session, NO persistence, NO event log — \
                 (input) → (computed report), deterministic + idempotent. Uses the SAME kernel \
                 compute as the session path (criticality matrix, residual, standing, \
                 response_class, readiness) so it cannot diverge. \
                 Args: optional matrix_strategy (qualitative3x3 (default) | nasa8004_5x5) + \
                 failure_modes[] — each {id, component{id}, description, cause?, effect?, domain, \
                 scope?, severity_observations[], probability_observations[], mitigations[]{id, \
                 kind, description, residual_severity_observations[], \
                 residual_probability_observations[]}}. ALL observation arrays (incl. the residual \
                 ones) are ids from scoring.catalog for the SELECTED strategy (NOT scores). \
                 Returns {matrix_strategy, failure_modes[]{id, criticality, residual_criticality, \
                 standing, response_class, issues[]}, risk_ranking[] (fm ids, highest risk first), \
                 issues[], ready, blockers[]}. An unknown observation id → INVALID_OBSERVATION.",
            ),
            analyze_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_SESSION_LIST),
            Cow::Borrowed(
                "List the analyses in the state dir (the project's .fmeca/ when there is one). \
                 Optional exact-match filters: level, aspect, parent. Returns {state_dir, \
                 sessions[]} sorted by session_id; each row is {session_id, title, level, \
                 aspect, scope_paths, parent, failure_modes (count, excluded ones left out), \
                 by_standing{unmitigated, under_mitigated, acceptable}, excluded (count), ready, ready_verified, resolution{planned, \
                 implemented, verified, abandoned}, critical_items (count), accepted_risks \
                 (count)}. A log that fails to replay is listed as {session_id, error} instead \
                 of failing the call (such rows are kept whatever the filters).",
            ),
            session_list_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_ACTIONS_SUGGEST),
            Cow::Borrowed(
                "READ-ONLY. Read an analysis under a declarative risk policy and SUGGEST actions; \
                 it never changes the analysis or appends to the log. Pass exactly one of \
                 session_id or analysis ({matrix_strategy?, failure_modes[]} as for analyze). \
                 Optional: policy (a preset name or a file name in .fmeca/policies/, no paths; \
                 default compat-0.1), as_of (YYYY-MM-DD; default today, UTC; echoed back), bands[] \
                 (keep failure modes in these effective bands), tags[] (keep actions with any of \
                 these tags), required_only (keep only required actions), group_by \
                 (failure_mode|band|tag|action). Roll-ups and signoff always cover the whole \
                 analysis. Returns {policy{name,hash,class,matrix_derived_from,source}, as_of, \
                 failure_modes[]{id, inherent_band, residual_band, effective_band, confidence, \
                 fallback?, status, rationale_needed, actions[]}, excluded[], rollups, \
                 signoff{pass,reasons[]}, groups?}. Errors: POLICY_NOT_FOUND, INVALID_POLICY, \
                 POLICY_STRATEGY_MISMATCH.",
            ),
            actions_suggest_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_POLICY_LIST),
            Cow::Borrowed(
                "READ-ONLY. List the risk policies actions.suggest accepts: {presets[], \
                 project[]}, each {name, hash, class, description, source}. Project policies are \
                 the files in .fmeca/policies/; one that fails to load is listed with an error \
                 instead of failing the call.",
            ),
            policy_list_schema(),
        ),
        #[cfg(feature = "jev")]
        Tool::new(
            Cow::Borrowed(TOOL_VET),
            Cow::Borrowed(
                "ADVISORY second opinion from Jev (a calibrated judgment model on OpenRouter; \
                 needs OPENROUTER_API_KEY). Pass exactly one of session_id or analysis (the \
                 analyze args shape). Asks up to max_questions (default 40, max 200) in four \
                 checks: observations (do the claimed scoring.catalog observations apply; does \
                 the next level up?), mitigations (how fully each addresses the cause; is its \
                 declared kind right?), coverage (failure classes no failure mode covers) and \
                 readiness (sound enough to sign off?). Returns {status: ok, model, usage{cost}, \
                 readiness_gate, findings[]{check, target, question_hash, probability, \
                 threshold, verdict (flag|ok; readiness concern|ok)}, skipped[] (unsent \
                 question hashes), questions_sent} or {status: vet_unavailable, reason \
                 (no_key|timeout|http_4xx|http_5xx|network|bad_response|bad_config), detail, \
                 readiness_gate}. NEVER writes and NEVER changes a score: the readiness_gate is \
                 the deterministic one; findings are probabilities to weigh, not verdicts. \
                 Excluded failure modes are skipped.",
            ),
            vet_schema(),
        ),
    ]
}

#[cfg(feature = "jev")]
fn vet_schema() -> Arc<rmcp::model::JsonObject> {
    let mut analysis = Value::Object((*analyze_schema()).clone());
    analysis["description"] =
        json!("A one-shot analysis in the analyze tool's argument shape. Not with session_id.");
    schema_object(json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "string",
                "description": "The session to vet. Pass this or analysis, not both."
            },
            "analysis": analysis,
            "checks": {
                "type": "array",
                "items": { "type": "string", "enum": crate::vet::CHECKS },
                "description": "Optional. Which checks to run; default all four. Readiness and coverage are always asked before the per-failure-mode checks."
            },
            "max_questions": {
                "type": "integer",
                "minimum": 0,
                "maximum": crate::vet::HARD_MAX_QUESTIONS,
                "description": "Optional. Questions to send (default 40); the rest are listed in skipped. 0 sends none."
            }
        },
        "additionalProperties": false
    }))
}

fn session_id_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": { "session_id": { "type": "string" } },
        "required": ["session_id"],
        "additionalProperties": false
    }))
}

/// The closed set of selectable matrix-strategy ids.
const MATRIX_STRATEGY_IDS: [&str; 2] = ["qualitative3x3", "nasa8004_5x5"];

/// The closed set of analysis levels (spec §6.2), in order.
const ANALYSIS_LEVELS: [&str; 6] = [
    "concept",
    "conops",
    "technical_plan",
    "project_plan",
    "implementation",
    "verification",
];

fn session_open_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "session_id": { "type": "string" },
            "matrix_strategy": {
                "type": "string",
                "enum": MATRIX_STRATEGY_IDS,
                "description": "Optional. Selects the criticality matrix; default qualitative3x3. \
                                Fixed once the session is opened."
            },
            "title": { "type": "string", "description": "Optional. A human title for the analysis." },
            "level": {
                "type": "string",
                "enum": ANALYSIS_LEVELS,
                "description": "Optional. The level of this analysis."
            },
            "aspect": {
                "type": "string",
                "description": "Optional. What is analysed, e.g. checkout."
            },
            "scope_paths": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional. Repo paths this analysis covers."
            },
            "parent": {
                "type": "string",
                "description": "Optional. The parent session id (must exist), e.g. a technical plan's conops."
            }
        },
        "required": ["session_id"],
        "additionalProperties": false
    }))
}

fn session_list_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "level": {
                "type": "string",
                "enum": ANALYSIS_LEVELS,
                "description": "Optional. Only analyses at this level."
            },
            "aspect": { "type": "string", "description": "Optional. Only analyses of this aspect." },
            "parent": { "type": "string", "description": "Optional. Only children of this session id." }
        },
        "additionalProperties": false
    }))
}

/// One resolution evidence item: where it is and, optionally, why it counts.
fn evidence_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["commit", "pull_request", "test", "file", "url", "note"]
                },
                "reference": { "type": "string" },
                "basis": {
                    "type": "string",
                    "enum": ["design", "test", "inspection", "failure_history", "operational_use"]
                }
            },
            "required": ["kind", "reference"],
            "additionalProperties": false
        }
    })
}

/// The `append` envelope: one flat object (no top-level `oneOf`) whose
/// `variant` decides which other keys apply. The kernel validates the deep
/// shapes and returns the stable error prefixes.
fn append_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "variant": {
                "type": "string",
                "enum": [
                    "add_failure_mode",
                    "add_mitigation",
                    "rescore",
                    "set_mitigation_status",
                    "set_failure_mode_disposition"
                ]
            },
            "session_id": { "type": "string" },
            "failure_mode": {
                "type": "object",
                "description": "add_failure_mode. Besides the core fields: optional local_effect, next_effect (effect is the end effect), single_point, parent{session_id, failure_mode_id} and refs[].",
                "properties": {
                    "local_effect": { "type": "string" },
                    "next_effect": { "type": "string" },
                    "single_point": { "type": "boolean" },
                    "parent": {
                        "type": "object",
                        "properties": {
                            "session_id": { "type": "string" },
                            "failure_mode_id": { "type": "string" }
                        },
                        "required": ["session_id", "failure_mode_id"]
                    },
                    "refs": { "type": "array", "items": { "type": "string" } }
                }
            },
            "mitigation": { "type": "object" },
            "rescore": { "type": "object" },
            "mitigation_id": { "type": "string", "description": "set_mitigation_status." },
            "status": {
                "type": "string",
                "enum": ["planned", "implemented", "verified", "abandoned"],
                "description": "set_mitigation_status. verified needs evidence whose kind is not note."
            },
            "note": { "type": "string", "description": "set_mitigation_status. Optional." },
            "failure_mode_id": { "type": "string", "description": "set_failure_mode_disposition." },
            "disposition": {
                "type": "string",
                "enum": ["open", "accepted", "excluded"],
                "description": "set_failure_mode_disposition. accepted needs an approver; excluded needs evidence whose kind is not note."
            },
            "rationale": { "type": "string", "description": "set_failure_mode_disposition. Required, non-empty." },
            "approver": { "type": "string", "description": "set_failure_mode_disposition. Required for accepted." },
            "evidence": evidence_schema()
        },
        "required": ["variant", "session_id"]
    }))
}

fn scoring_catalog_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "matrix_strategy": {
                "type": "string",
                "enum": MATRIX_STRATEGY_IDS,
                "description": "Optional. Which strategy's catalog to return; default qualitative3x3."
            }
        },
        "additionalProperties": false
    }))
}

/// Schema for the stateless `analyze` batch tool. Documents the batch envelope;
/// the kernel validates the deep failure-mode/mitigation shapes and returns the
/// stable error prefixes (e.g. INVALID_OBSERVATION).
fn analyze_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "matrix_strategy": {
                "type": "string",
                "enum": MATRIX_STRATEGY_IDS,
                "description": "Optional. Selects the criticality matrix; default qualitative3x3."
            },
            "failure_modes": {
                "type": "array",
                "description": "The entire batch of failure modes to analyze in one shot.",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "component": {
                            "type": "object",
                            "properties": { "id": { "type": "string" } },
                            "required": ["id"]
                        },
                        "description": { "type": "string" },
                        "cause": { "type": "string" },
                        "effect": { "type": "string" },
                        "domain": {
                            "type": "string",
                            "enum": ["ux", "runtime", "architecture", "delivery", "security"]
                        },
                        "scope": {
                            "type": "string",
                            "enum": ["localized", "cross_cutting", "structural"]
                        },
                        "severity_observations": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "scoring.catalog ids on the severity axis for the selected strategy."
                        },
                        "probability_observations": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "scoring.catalog ids on the probability axis for the selected strategy."
                        },
                        "mitigations": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "kind": {
                                        "type": "string",
                                        "enum": ["prevention", "detection", "fail_fast"]
                                    },
                                    "description": { "type": "string" },
                                    "residual_severity_observations": {
                                        "type": "array",
                                        "items": { "type": "string" },
                                        "description": "scoring.catalog ids on the severity axis (the residual severity AFTER this mitigation). The engine derives the level; you NEVER supply a score."
                                    },
                                    "residual_probability_observations": {
                                        "type": "array",
                                        "items": { "type": "string" },
                                        "description": "scoring.catalog ids on the probability axis (the residual probability AFTER this mitigation)."
                                    }
                                },
                                "required": ["id", "kind", "description"]
                            }
                        }
                    },
                    "required": ["id", "component", "description", "domain"]
                }
            }
        },
        "required": ["failure_modes"],
        "additionalProperties": false
    }))
}

/// Schema for `actions.suggest`: one flat object, no top-level
/// `oneOf`/`anyOf`/`allOf`; the handler enforces exactly one of
/// `session_id` / `analysis`.
fn actions_suggest_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {
            "session_id": {
                "type": "string",
                "description": "A session to read. Pass exactly one of session_id and analysis."
            },
            "analysis": {
                "type": "object",
                "description": "A one-shot analysis, the same envelope as the analyze tool. Pass exactly one of session_id and analysis.",
                "properties": {
                    "matrix_strategy": { "type": "string", "enum": MATRIX_STRATEGY_IDS },
                    "failure_modes": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["failure_modes"]
            },
            "policy": {
                "type": "string",
                "description": "A preset name or a file name in .fmeca/policies/ (no paths, no ..). Default compat-0.1."
            },
            "as_of": {
                "type": "string",
                "description": "YYYY-MM-DD. Default: today (UTC). Echoed back."
            },
            "bands": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Keep only failure modes whose effective band is listed."
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Keep only actions carrying any of these tags."
            },
            "required_only": {
                "type": "boolean",
                "description": "Keep only required actions."
            },
            "group_by": {
                "type": "string",
                "enum": ["failure_mode", "band", "tag", "action"],
                "description": "Add a groups map."
            }
        },
        "additionalProperties": false
    }))
}

fn policy_list_schema() -> Arc<rmcp::model::JsonObject> {
    schema_object(json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    }))
}

fn instructions() -> String {
    let mut text = BASE_INSTRUCTIONS.to_string();
    if cfg!(feature = "jev") {
        text.push_str(VET_INSTRUCTIONS);
    }
    text
}

const VET_INSTRUCTIONS: &str = r#"
VET (advisory): vet asks Jev, a calibrated judgment model on OpenRouter, about a session
or a one-shot analysis — do the claimed observations apply, do mitigations address their
causes and match their declared kind, which failure classes are uncovered, is it ready to
sign off — and reports probabilities beside the deterministic readiness gate. It needs
OPENROUTER_API_KEY (otherwise status vet_unavailable, reason no_key). The probabilities
are a second opinion: they NEVER change a score, an observation or the gate, and vet never
appends. Act on a flag by revisiting the evidence and appending through the normal tools.
"#;

const BASE_INSTRUCTIONS: &str = r#"fmeca-mcp — a deterministic, offline structured-FMECA engine.

You (the caller, an LLM) do the fuzzy work: naming failure modes, causes, effects,
and proposing mitigations. This server owns STRUCTURE: a typed failure-mode /
mitigation ledger, the fixed qualitative criticality matrix (S×P → High/Medium/Low),
the prevent→detect→fail-fast mitigation-order discipline, computed residual risk,
gap detection (notify/clarify/remediate signals), a readiness gate, and report export.

Criticality, residual, and standing are COMPUTED (a pure fold over an append-only
log), never stored. No LLM, no network in the kernel.

Tools (the eight core tools: command/query split + one stateless batch; plus session.list):
  session.open      — start/resume a session; optional matrix_strategy SELECTS the matrix;
                      optional title, level, aspect, scope_paths, parent (fixed once opened);
                      returns FmecaState (incl. meta, matrix_strategy, matrix_scale, scoring_catalog)
  append            — variant add_failure_mode | add_mitigation | rescore | set_mitigation_status |
                      set_failure_mode_disposition; returns state + signals
  state.get         — read-only FmecaState projection (incl. matrix_strategy + scoring_catalog)
  risk.next         — highest-criticality unmitigated failure mode (or {risk: null})
  readiness.assess  — ReadinessReport (ready iff all scored + no residual High/Medium +
                      no weak_mitigation_order issue)
  report.export     — the FMECA table report (rows + residual buckets + blockers)
  scoring.catalog   — the fixed evidence→score catalog (the observation vocabulary) for a
                      strategy (optional matrix_strategy arg; default qualitative3x3)
  analyze           — STATELESS one-shot FMECA: pass the ENTIRE batch of failure modes (with
                      inlined mitigations) + optional matrix_strategy, get back the ENTIRE
                      computed report (per-FM criticality/residual/standing/response_class +
                      issues, risk_ranking, ready, blockers). No session, no persistence —
                      deterministic + idempotent, same compute as the session path.
  session.list      — every analysis in the state dir with its metadata, standing counts,
                      ready, ready_verified and resolution counts (filters: level, aspect, parent)

POLICIES (read-only): actions.suggest reads an analysis under a declarative policy and suggests
actions; it never changes the analysis. policy.list shows the presets and .fmeca/policies/ files.

PROJECT STORE AND RESOLUTION:
  Analyses live in the project's .fmeca/ folder, one <session_id>.jsonl per analysis.
  Use session.list to find them before opening a new one.
  Track resolution with append set_mitigation_status (planned → implemented → verified, with evidence).
  Record risk acceptance or a not-credible exclusion with append set_failure_mode_disposition.
  Never delete a failure mode.

MATRIX STRATEGY: the criticality matrix is a SWAPPABLE, CLOSED strategy you
SELECT per session at session.open via matrix_strategy — you NEVER edit its cells:
  qualitative3x3 (default) — the 3-level Low/Medium/High matrix.
  nasa8004_5x5             — the NASA GSFC-HDBK-8004 5×5 matrix: 5-level consequence +
                             5-level likelihood, all 25 cells collapsed to Low/Medium/High.
A session's strategy is fixed once opened and recorded for deterministic replay. Each
strategy has its OWN scoring.catalog (its observation vocabulary) and its own scale —
an observation id valid under one strategy is unknown under another (INVALID_OBSERVATION).
The criticality OUTPUT is always low|medium|high, so readiness / response_class are
unchanged whichever strategy you pick.

CRITICAL: you NEVER supply a severity/probability score — including the
RESIDUAL after a mitigation. You supply OBSERVATIONS — ids from the ACTIVE strategy's
scoring.catalog — as severity_observations / probability_observations on a failure_mode /
rescore, AND as residual_severity_observations / residual_probability_observations on a
mitigation. The ENGINE maps observations to a strategy-relative level by the SAME fixed
code-resident map (MAX-combine: the worst observed evidence wins), then the matrix collapses
it to criticality (low | medium | high) — the residual uses the SAME derivation as the
unmitigated axes. An empty observation set leaves that axis unscored (missing_score); an
unknown id is rejected (INVALID_OBSERVATION).

Each failure mode also gets a deterministic response_class (re_architecture | restructure |
minor_fix) from its criticality + domain + optional scope (localized | cross_cutting |
structural) — the magnitude of the fix.

Supply CANONICAL identity: the component is an EntityRef{id}; failure-mode and
mitigation ids are caller-assigned and share one flat namespace (duplicates rejected).
A mitigation's residual S/P is observation-derived too (residual_severity_observations /
residual_probability_observations) — never a level you pick.
Mitigation kinds: prevention | detection | fail_fast (prevention preferred).
Domains: ux | runtime | architecture | delivery | security. Every fact carries a source
EvidenceRef{turn_id}. Errors carry stable prefixes: SESSION_NOT_FOUND, BAD_SESSION_ID,
INVALID_FAILURE_MODE, INVALID_MITIGATION, INVALID_RESCORE, INVALID_OBSERVATION,
FM_NOT_FOUND, DUPLICATE_ID, STORE_ERROR.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_instructions_do_not_mention_vet() {
        assert!(!BASE_INSTRUCTIONS.to_lowercase().contains("vet"));
    }
}
