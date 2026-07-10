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
        .route("/style.css", get(style_css))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/{id}/summary", get(provider_summary))
        .route("/api/providers/{id}/op/{opid}", post(run_operation))
        .route("/api/providers/{id}/install", post(start_install))
        .route("/api/providers/{id}/update-check", post(update_check))
        .route("/api/providers/{id}", delete(delete_provider))
        .route("/api/install/{task}", get(install_status))
        .route("/api/register", post(register_provider))
        .layer(middleware::from_fn(require_local_host))
        .with_state(app)
}

/// Reject requests whose Host is not a loopback name (DNS-rebinding guard).
async fn require_local_host(
    req: axum::extract::Request,
    next: middleware::Next,
) -> Response {
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

async fn style_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], include_str!("assets/style.css"))
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
        return err(StatusCode::BAD_REQUEST, format!("{id} has no summary query"));
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
    Json(json!({
        "state": "ok",
        "balance": fields.balance,
        "due_date": fields.due_date,
        "raw": out.stdout,
    }))
    .into_response()
}

async fn run_operation(Path((id, opid)): Path<(String, String)>) -> Response {
    let Some(p) = find_provider(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no provider {id}"));
    };
    let Some(op) = p.manifest.operations.iter().find(|o| o.id == opid) else {
        return err(StatusCode::NOT_FOUND, format!("{id} has no operation {opid}"));
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(StatusCode::CONFLICT, format!("{} is not installed", p.manifest.binary));
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
    let self_update = find_binary(&p.manifest.binary).zip(spec.self_update_args.clone());
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
    let Some(args) = p.manifest.install.as_ref().and_then(|i| i.update_check_args.clone()) else {
        return err(StatusCode::BAD_REQUEST, format!("{id} has no update check"));
    };
    let Some(bin) = find_binary(&p.manifest.binary) else {
        return err(StatusCode::CONFLICT, format!("{} is not installed", p.manifest.binary));
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
            format!("provider {} already exists (set overwrite to replace)", m.id),
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
        return err(StatusCode::BAD_REQUEST, format!("{id} is built in and can't be removed"));
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
        let start = (cut..s.len()).find(|i| s.is_char_boundary(*i)).unwrap_or(cut);
        format!("…{}", &s[start..])
    }
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn tail_respects_char_boundaries() {
        let s = "héllo wörld";
        let t = tail(s, 4);
        assert!(t.starts_with('…'));
        assert!(t.len() <= 8);
    }
}
