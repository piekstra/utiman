//! The local HTTP server: embedded dashboard assets plus a JSON API.
//!
//! Security model: this server executes provider CLIs, so it binds to
//! 127.0.0.1 only and additionally rejects any request whose Host header is
//! not local. That second check matters — DNS rebinding can point a public
//! name at 127.0.0.1 and let a web page you visit reach a localhost server;
//! the Host check shuts that down.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{middleware, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::detect::{detect, find_binary};
use crate::extract::extract_summary;
use crate::install::{install_argv, InstallTasks};
use crate::manifest::{self, load_providers, parse_manifest, Provider, Source};
use crate::runner::{run_cli, RunOutcome, DEFAULT_TIMEOUT_SECS};

pub struct App {
    pub tasks: Arc<InstallTasks>,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/charts.js", get(charts_js))
        .route("/style.css", get(style_css))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/{id}/summary", get(provider_summary))
        .route("/api/providers/{id}/op/{opid}", post(run_operation))
        .route("/api/providers/{id}/pay", post(open_pay))
        .route("/api/providers/{id}/install", post(start_install))
        .route("/api/providers/{id}/update-check", post(update_check))
        .route("/api/providers/{id}/auth-status", get(auth_status))
        .route("/api/providers/{id}/login-terminal", post(login_terminal))
        .route("/api/providers/{id}/setup/{setup_id}", post(run_setup))
        .route("/api/providers/{id}/snapshots", get(snapshots_for))
        .route("/api/providers/{id}/series/{sid}", get(series_data))
        .route("/api/providers/{id}/doc/{docid}", get(download_document))
        .route("/api/providers/{id}", delete(delete_provider))
        .route("/api/install/{task}", get(install_status))
        .route("/api/register", post(register_provider))
        .layer(middleware::from_fn(require_local_host))
        .with_state(app)
}

/// Reject requests whose Host is not a loopback name (DNS-rebinding guard).
async fn require_local_host(req: axum::extract::Request, next: middleware::Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    // Strip :port — for IPv6 literals the bracket form [::1]:port.
    let bare = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    if matches!(bare, "localhost" | "127.0.0.1" | "::1") {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "utiman only answers local requests").into_response()
    }
}

async fn index() -> Html<&'static str> {
    Html(include_str!("assets/index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("assets/app.js"),
    )
}

async fn charts_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        include_str!("assets/charts.js"),
    )
}

async fn style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css")],
        include_str!("assets/style.css"),
    )
}

fn find_provider(id: &str) -> Option<Provider> {
    load_providers().into_iter().find(|p| p.manifest.id == id)
}

#[derive(Serialize)]
struct ProviderInfo {
    #[serde(flatten)]
    manifest: manifest::Manifest,
    source: Source,
    detection: crate::detect::Detection,
    installing: bool,
}

async fn list_providers(State(app): State<Arc<App>>) -> Json<Value> {
    let mut out = Vec::new();
    for p in load_providers() {
        let detection = detect(&p.manifest.binary).await;
        let installing = app.tasks.running_for(&p.manifest.id);
        out.push(ProviderInfo {
            manifest: p.manifest,
            source: p.source,
            detection,
            installing,
        });
    }
    Json(json!({ "providers": out }))
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// Run the manifest's summary query and normalize it for a dashboard card.
async fn provider_summary(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let m = &p.manifest;
    let Some(query) = &m.summary else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("{id} has no summary query"),
        );
    };
    let Some(bin) = find_binary(&m.binary) else {
        return Json(json!({ "state": "not-installed" })).into_response();
    };

    let out = run_cli(&bin, &query.args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await;
    if !out.ok() {
        let hint = m
            .auth
            .as_ref()
            .filter(|a| a.required)
            .and_then(|a| a.login_command.clone())
            .or_else(|| m.setup_command.clone());
        return Json(json!({
            "state": "error",
            "stderr": tail(&out.stderr, 2000),
            "timed_out": out.timed_out,
            "hint": hint,
        }))
        .into_response();
    }

    let fields = extract_summary(query, &out.stdout);
    if let Some(balance) = fields.balance {
        crate::snapshots::record(&id, balance, fields.due_date.as_deref());
    }
    Json(json!({
        "state": "ok",
        "balance": fields.balance,
        "due_date": fields.due_date,
        "raw": out.stdout,
    }))
    .into_response()
}

async fn snapshots_for(Path(id): Path<String>) -> Response {
    if find_provider(&id).is_none() {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    }
    Json(json!({ "snapshots": crate::snapshots::read(&id) })).into_response()
}

/// Run a manifest series command and return chart-ready points.
async fn series_data(Path((id, sid)): Path<(String, String)>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(series) = p.manifest.series.iter().find(|s| s.id == sid) else {
        return err(StatusCode::NOT_FOUND, format!("{id} has no series {sid}"));
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let out = run_cli(
        &bin,
        &series.args,
        Duration::from_secs(DEFAULT_TIMEOUT_SECS),
    )
    .await;
    if !out.ok() {
        return Json(json!({
            "ok": false,
            "stderr": tail(&out.stderr, 2000),
            "timed_out": out.timed_out,
        }))
        .into_response();
    }
    let fresh = crate::extract::extract_series(series, &out.stdout);
    // Fold this fetch into the local archive and return the merged history, so
    // charts extend past the CLI's own window the longer utiman runs.
    let points = crate::archive::merge(&id, &series.id, &fresh);
    Json(json!({
        "ok": true,
        "points": points,
        "archived": points.len() > fresh.len(),
        "unit": series.unit,
        "chart": series.chart,
        "name": series.name,
    }))
    .into_response()
}

/// Run a document command with a temp output path and stream the file back.
async fn download_document(Path((id, docid)): Path<(String, String)>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(doc) = p.manifest.documents.iter().find(|d| d.id == docid) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("{id} has no document {docid}"),
        );
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let tmp = std::env::temp_dir().join(format!(
        "utiman-{}-{}-{}",
        p.manifest.id,
        std::process::id(),
        doc.filename
    ));
    let mut args = doc.args.clone();
    args.push(doc.out_flag.clone());
    args.push(tmp.display().to_string());
    let out = run_cli(&bin, &args, Duration::from_secs(120)).await;
    let bytes = fs::read(&tmp);
    let _ = fs::remove_file(&tmp);
    if !out.ok() {
        return err(
            StatusCode::BAD_GATEWAY,
            format!("document command failed: {}", tail(&out.stderr, 500)),
        );
    }
    let Ok(bytes) = bytes else {
        return err(
            StatusCode::BAD_GATEWAY,
            "command succeeded but produced no file",
        );
    };
    let content_type = match doc.filename.rsplit('.').next() {
        Some("pdf") => "application/pdf",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    };
    (
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", doc.filename),
            ),
        ],
        bytes,
    )
        .into_response()
}

async fn run_operation(Path((id, opid)): Path<(String, String)>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(op) = p.manifest.operations.iter().find(|o| o.id == opid) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("{id} has no operation {opid}"),
        );
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let out: RunOutcome = run_cli(&bin, &op.args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await;
    Json(json!({
        "ok": out.ok(),
        "status": out.status,
        "stdout": out.stdout,
        "stderr": tail(&out.stderr, 4000),
        "timed_out": out.timed_out,
    }))
    .into_response()
}

/// Hand off to the provider's official pay page by running its `pay.open-args`
/// (the CLI opens the browser). The `url` form is opened client-side, so this
/// only handles the CLI hand-off. utiman never sees card data either way.
async fn open_pay(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(args) = p
        .manifest
        .pay
        .as_ref()
        .and_then(|pay| pay.open_args.clone())
    else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("{id} has no pay open-args"),
        );
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let out = run_cli(&bin, &args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await;
    Json(json!({
        "ok": out.ok(),
        "stdout": out.stdout,
        "stderr": tail(&out.stderr, 2000),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SetupBody {
    value: String,
}

/// Apply a non-secret setup input: run `<binary> <args...> <value>`. The value
/// is passed as a single trailing argv element (no shell), and setup inputs are
/// declared only for non-secret values — credentials never reach this path.
async fn run_setup(
    Path((id, setup_id)): Path<(String, String)>,
    Json(body): Json<SetupBody>,
) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(input) = p.manifest.setup.iter().find(|s| s.id == setup_id) else {
        return err(
            StatusCode::NOT_FOUND,
            format!("{id} has no setup {setup_id}"),
        );
    };
    let value = body.value.trim();
    if value.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            format!("{} requires a value", input.name),
        );
    }
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let mut args = input.args.clone();
    args.push(value.to_string());
    let out = run_cli(&bin, &args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await;
    Json(json!({
        "ok": out.ok(),
        "stdout": out.stdout,
        "stderr": tail(&out.stderr, 2000),
        "timed_out": out.timed_out,
    }))
    .into_response()
}

/// Install or update. When the CLI is already installed and its manifest
/// declares self-update args, the CLI updates itself; otherwise this falls
/// back to the manifest's install command (idempotent, `--force`).
async fn start_install(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(spec) = &p.manifest.install else {
        return err(StatusCode::BAD_REQUEST, format!("{id} has no install spec"));
    };
    // Conforming CLIs default to the standard `self-update` spelling.
    let self_update_args = spec
        .self_update_args
        .clone()
        .unwrap_or_else(|| vec!["self-update".into()]);
    let self_update = find_binary(&p.manifest.binary).map(|b| (b, self_update_args));
    let argv = match self_update {
        Some((bin, args)) => {
            let mut v = vec![bin.display().to_string()];
            v.extend(args);
            Ok(v)
        }
        None => install_argv(spec),
    };
    let argv = match argv {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };
    match app.tasks.start(&id, argv) {
        Ok(task) => Json(json!({ "task": task })).into_response(),
        Err(e) => err(StatusCode::CONFLICT, e.to_string()),
    }
}

/// Run the manifest's update-check args (e.g. `lrfl self-update --check`).
async fn update_check(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(install) = p.manifest.install.as_ref() else {
        return err(StatusCode::BAD_REQUEST, format!("{id} has no update check"));
    };
    let args = install
        .update_check_args
        .clone()
        .unwrap_or_else(|| vec!["self-update".into(), "--check".into(), "--json".into()]);
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(
            StatusCode::CONFLICT,
            format!("{} is not installed", p.manifest.binary),
        );
    };
    let out = run_cli(&bin, &args, Duration::from_secs(DEFAULT_TIMEOUT_SECS)).await;
    Json(json!({
        "ok": out.ok(),
        "stdout": out.stdout,
        "stderr": tail(&out.stderr, 2000),
    }))
    .into_response()
}

async fn install_status(State(app): State<Arc<App>>, Path(task): Path<u64>) -> Response {
    let Some(t) = app.tasks.get(task) else {
        return err(StatusCode::NOT_FOUND, "no such install task");
    };
    let state = *t.state.lock().unwrap();
    let log = t.log.lock().unwrap().clone();
    Json(json!({ "state": state, "log": log })).into_response()
}

/// Report whether the provider's CLI is signed in, using the manifest's
/// status args + `authenticated-field` dot-path. The CLIs exit 0 either way
/// (status is a report, not a gate), so the answer comes from the JSON.
async fn auth_status(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    // Conforming CLIs (piekstra-cli/1) need no per-provider config: default
    // to `auth status --json` and its canonical `authenticated` field.
    let Some(auth) = p.manifest.auth.as_ref() else {
        return Json(json!({ "state": "unknown" })).into_response();
    };
    let args = auth
        .status_args
        .clone()
        .unwrap_or_else(|| vec!["auth".into(), "status".into(), "--json".into()]);
    let field = auth
        .authenticated_field
        .clone()
        .unwrap_or_else(|| "authenticated".into());
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return Json(json!({ "state": "unknown" })).into_response();
    };
    let out = run_cli(&bin, &args, Duration::from_secs(15)).await;
    if !out.ok() {
        return Json(json!({ "state": "unknown", "stderr": tail(&out.stderr, 500) }))
            .into_response();
    }
    let state = serde_json::from_str::<Value>(&out.stdout)
        .ok()
        .and_then(|v| crate::extract::json_path(&v, &field).map(truthy))
        .map(|authed| {
            if authed {
                "authenticated"
            } else {
                "unauthenticated"
            }
        })
        .unwrap_or("unknown");
    Json(json!({ "state": state })).into_response()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::Null => false,
        _ => true,
    }
}

/// Open the user's terminal with the provider's interactive login command.
/// The credentials still flow only between the user's terminal and the CLI —
/// utiman just launches the window.
async fn login_terminal(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(cmd) = p
        .manifest
        .auth
        .as_ref()
        .and_then(|a| a.login_command.clone())
    else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("{id} has no login command"),
        );
    };
    if std::env::consts::OS != "macos" {
        return err(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "opening a terminal isn't wired up on this platform yet — run `{cmd}` yourself"
            ),
        );
    }
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        applescript_escape(&cmd)
    );
    let out = run_cli(
        std::path::Path::new("/usr/bin/osascript"),
        &["-e".to_string(), script],
        Duration::from_secs(10),
    )
    .await;
    if out.ok() {
        Json(json!({ "opened": true, "command": cmd })).into_response()
    } else {
        err(StatusCode::INTERNAL_SERVER_ERROR, tail(&out.stderr, 500))
    }
}

fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Deserialize)]
struct RegisterBody {
    toml: String,
    #[serde(default)]
    overwrite: bool,
}

/// Save a user manifest into ~/.config/utiman/providers/<id>.toml.
async fn register_provider(Json(body): Json<RegisterBody>) -> Response {
    let m = match parse_manifest(&body.toml) {
        Ok(m) => m,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let exists = load_providers().iter().any(|p| p.manifest.id == m.id);
    if exists && !body.overwrite {
        return err(
            StatusCode::CONFLICT,
            format!(
                "provider {} already exists (set overwrite to replace)",
                m.id
            ),
        );
    }
    let dir = manifest::user_providers_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let path = dir.join(format!("{}.toml", m.id));
    if let Err(e) = fs::write(&path, &body.toml) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    Json(json!({ "id": m.id, "path": path.display().to_string() })).into_response()
}

/// Remove a user-registered manifest. Built-ins can't be deleted (they can be
/// shadowed by a user manifest with the same id instead).
async fn delete_provider(Path(id): Path<String>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(path) = (p.source == Source::User).then_some(p.path).flatten() else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("{id} is built in and can't be removed"),
        );
    };
    match fs::remove_file(&path) {
        Ok(()) => Json(json!({ "removed": id })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let cut = s.len() - max;
        let start = (cut..s.len())
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(cut);
        format!("…{}", &s[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::{applescript_escape, tail, truthy};
    use serde_json::json;

    #[test]
    fn tail_respects_char_boundaries() {
        let s = "héllo wörld";
        let t = tail(s, 4);
        assert!(t.starts_with('…'));
        assert!(t.len() <= 8);
    }

    #[test]
    fn truthy_covers_status_shapes() {
        assert!(truthy(&json!(true)));
        assert!(!truthy(&json!(false)));
        assert!(!truthy(&json!(null)));
        assert!(truthy(&json!("user@example.com")));
        assert!(!truthy(&json!("")));
        assert!(!truthy(&json!(0)));
    }

    #[test]
    fn applescript_escaping() {
        assert_eq!(applescript_escape(r#"echo "a\b""#), r#"echo \"a\\b\""#);
    }
}
