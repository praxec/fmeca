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
//! | `session.open`     | command | [`Engine::open_session`]       |
//! | `append`           | command | `add_failure_mode`/`add_mitigation`/`rescore` |
//! | `state.get`        | query   | [`Engine::state`]              |
//! | `risk.next`        | query   | [`Engine::risk_next`]          |
//! | `readiness.assess` | query   | [`Engine::readiness`]          |
//! | `report.export`    | query   | [`Engine::export`]             |
//! | `scoring.catalog`  | query   | [`fmeca::catalog`]        |
//! | `analyze`          | batch   | [`fmeca::analyze`]        |
//!
//! Errors propagate the kernel's stable prefixes (`SESSION_NOT_FOUND`,
//! `INVALID_FAILURE_MODE`, `BAD_SESSION_ID`, …) verbatim in the MCP error
//! message.

use std::borrow::Cow;
use std::sync::Arc;

use fmeca::{
    AnalyzeInput, Engine, FailureMode, FmecaError, MatrixStrategy, Mitigation, Rescore,
    ScoreCriterion,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::ErrorData as McpError;
use rmcp::{ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Tool names, `noun.verb` like the rest of the workspace.
pub const TOOL_SESSION_OPEN: &str = "session.open";
pub const TOOL_APPEND: &str = "append";
pub const TOOL_STATE_GET: &str = "state.get";
pub const TOOL_RISK_NEXT: &str = "risk.next";
pub const TOOL_READINESS_ASSESS: &str = "readiness.assess";
pub const TOOL_REPORT_EXPORT: &str = "report.export";
pub const TOOL_SCORING_CATALOG: &str = "scoring.catalog";
pub const TOOL_ANALYZE: &str = "analyze";

/// All tool names in declaration order. The six core command/query tools, the
/// `scoring.catalog` query (the fixed observation vocabulary the caller draws
/// from), and the stateless one-shot `analyze` batch tool.
pub const TOOL_NAMES: &[&str] = &[
    TOOL_SESSION_OPEN,
    TOOL_APPEND,
    TOOL_STATE_GET,
    TOOL_RISK_NEXT,
    TOOL_READINESS_ASSESS,
    TOOL_REPORT_EXPORT,
    TOOL_SCORING_CATALOG,
    TOOL_ANALYZE,
];

// --- wire arg structs ------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionIdArgs {
    session_id: String,
}

/// `session.open` args: a session id plus an OPTIONAL, closed
/// `matrix_strategy` selection. Defaults to the 3×3 strategy (back-compat). The
/// caller SELECTS a strategy; it cannot edit cells.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionOpenArgs {
    session_id: String,
    #[serde(default)]
    matrix_strategy: MatrixStrategy,
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
    server_name: String,
    server_version: String,
}

impl FmecaServer {
    /// Build a server backed by the supplied engine.
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            server_name: "fmeca-mcp".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        }
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

    /// Transport-free dispatch entry point (tests call this directly).
    pub fn dispatch_call(&self, request: CallToolRequestParams) -> Result<Value, McpError> {
        let args: Value = request
            .arguments
            .as_ref()
            .map(|m| Value::Object(m.clone()))
            .unwrap_or_else(|| json!({}));

        match request.name.as_ref() {
            TOOL_SESSION_OPEN => self.handle_session_open(args),
            TOOL_APPEND => self.handle_append(args),
            TOOL_STATE_GET => self.handle_state_get(args),
            TOOL_RISK_NEXT => self.handle_risk_next(args),
            TOOL_READINESS_ASSESS => self.handle_readiness(args),
            TOOL_REPORT_EXPORT => self.handle_export(args),
            TOOL_SCORING_CATALOG => self.handle_scoring_catalog(args),
            TOOL_ANALYZE => self.handle_analyze(args),
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
        let state = self
            .engine
            .open_session_with(&parsed.session_id, parsed.matrix_strategy)
            .map_err(engine_error_to_mcp)?;
        to_value(&state)
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
    fn get_info(&self) -> ServerInfo {
        let mut server_info =
            Implementation::new(self.server_name.clone(), self.server_version.clone());
        server_info.title = Some("fmeca-mcp".to_string());
        server_info.description =
            Some("Deterministic, offline structured-FMECA engine over MCP.".to_string());

        let mut info = InitializeResult::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info.instructions = Some(instructions().to_string());
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
    ) -> Result<CallToolResult, McpError> {
        self.dispatch_call(request).map(CallToolResult::structured)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_definitions().into_iter().find(|t| t.name == name)
    }

    async fn on_initialized(&self, _context: NotificationContext<RoleServer>) {
        tracing::info!("fmeca-mcp client initialized");
    }
}

// --- helpers ---------------------------------------------------------------

fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, McpError> {
    serde_json::from_value(args)
        .map_err(|e| McpError::invalid_params(format!("invalid arguments: {e}"), None))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, McpError> {
    serde_json::to_value(value)
        .map_err(|e| McpError::internal_error(format!("response serialisation failed: {e}"), None))
}

/// Map a kernel error to an MCP error, preserving the stable prefix in the
/// message. `BAD_SESSION_ID` / `INVALID_*` / `DUPLICATE_ID` are caller bugs
/// (`invalid_params`); the rest are surfaced as `internal_error`.
fn engine_error_to_mcp(err: FmecaError) -> McpError {
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

fn schema_object(value: Value) -> Arc<rmcp::model::JsonObject> {
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
                 cells. Returns the current FmecaState including matrix_strategy, matrix_scale, \
                 the active scoring_catalog, the component registry, computed criticality, \
                 signals, and readiness.",
            ),
            session_open_schema(),
        ),
        Tool::new(
            Cow::Borrowed(TOOL_APPEND),
            Cow::Borrowed(
                "Append one event. Tagged by `variant`: \
                 add_failure_mode{session_id, failure_mode} | \
                 add_mitigation{session_id, mitigation} | \
                 rescore{session_id, rescore}. \
                 A failure_mode / rescore supplies severity_observations + \
                 probability_observations, and a mitigation supplies \
                 residual_severity_observations + residual_probability_observations \
                 (all ids from scoring.catalog) — NOT a score; the engine derives every level. \
                 Returns the recomputed FmecaState with freshly-computed criticality + signals.",
            ),
            schema_object(json!({
                "type": "object",
                "properties": {
                    "variant": {
                        "type": "string",
                        "enum": ["add_failure_mode", "add_mitigation", "rescore"]
                    },
                    "session_id": { "type": "string" },
                    "failure_mode": { "type": "object" },
                    "mitigation": { "type": "object" },
                    "rescore": { "type": "object" }
                },
                "required": ["variant", "session_id"]
            })),
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
    ]
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
            }
        },
        "required": ["session_id"],
        "additionalProperties": false
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
                            "enum": ["ux", "runtime", "architecture", "delivery"]
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

fn instructions() -> &'static str {
    r#"fmeca-mcp — a deterministic, offline structured-FMECA engine.

You (the caller, an LLM) do the fuzzy work: naming failure modes, causes, effects,
and proposing mitigations. This server owns STRUCTURE: a typed failure-mode /
mitigation ledger, the fixed qualitative criticality matrix (S×P → High/Medium/Low),
the prevent→detect→fail-fast mitigation-order discipline, computed residual risk,
gap detection (notify/clarify/remediate signals), a readiness gate, and report export.

Criticality, residual, and standing are COMPUTED (a pure fold over an append-only
log), never stored. No LLM, no network in the kernel.

Tools (eight; command/query split + one stateless batch):
  session.open      — start/resume a session; optional matrix_strategy SELECTS the matrix;
                      returns FmecaState (incl. matrix_strategy, matrix_scale, scoring_catalog)
  append            — variant add_failure_mode | add_mitigation | rescore; returns state + signals
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
Domains: ux | runtime | architecture | delivery. Every fact carries a source
EvidenceRef{turn_id}. Errors carry stable prefixes: SESSION_NOT_FOUND, BAD_SESSION_ID,
INVALID_FAILURE_MODE, INVALID_MITIGATION, INVALID_RESCORE, INVALID_OBSERVATION,
FM_NOT_FOUND, DUPLICATE_ID, STORE_ERROR.
"#
}
