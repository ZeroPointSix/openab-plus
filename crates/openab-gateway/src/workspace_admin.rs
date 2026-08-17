use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const DEFAULT_WORKSPACE_ROOT: &str = "workspace";
const MAX_FILE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    root: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceFileDocument {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug)]
enum WorkspaceError {
    InvalidPath,
    NotFound,
    TooLarge,
    Io(std::io::Error),
}

impl From<std::io::Error> for WorkspaceError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl WorkspaceManager {
    pub fn from_env() -> Arc<Self> {
        let root = std::env::var("OPENAB_WORKSPACE_ROOT")
            .unwrap_or_else(|_| DEFAULT_WORKSPACE_ROOT.to_string());
        Arc::new(Self { root: root.into() })
    }

    #[cfg(test)]
    fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    async fn list(&self) -> Result<Vec<WorkspaceFile>, WorkspaceError> {
        tokio::fs::create_dir_all(&self.root).await?;
        let mut pending = vec![self.root.clone()];
        let mut files = Vec::new();
        while let Some(dir) = pending.pop() {
            let mut entries = tokio::fs::read_dir(&dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    pending.push(entry.path());
                } else if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES as u64 {
                    let relative = entry
                        .path()
                        .strip_prefix(&self.root)
                        .map_err(|_| WorkspaceError::InvalidPath)?
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.push(WorkspaceFile {
                        path: relative,
                        size: metadata.len(),
                    });
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    async fn read(&self, relative: &str) -> Result<WorkspaceFileDocument, WorkspaceError> {
        let path = self.resolve(relative).await?;
        let metadata = tokio::fs::metadata(&path).await.map_err(map_not_found)?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(WorkspaceError::TooLarge);
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(map_not_found)?;
        Ok(WorkspaceFileDocument {
            path: relative.to_string(),
            content,
        })
    }

    async fn write(&self, relative: &str, content: &str) -> Result<(), WorkspaceError> {
        if content.len() > MAX_FILE_BYTES {
            return Err(WorkspaceError::TooLarge);
        }
        let path = self.resolve(relative).await?;
        let parent = path.parent().ok_or(WorkspaceError::InvalidPath)?;
        tokio::fs::create_dir_all(parent).await?;
        self.reject_symlink_components(parent).await?;
        let tmp = parent.join(format!(".openab-write-{}.tmp", uuid::Uuid::new_v4()));
        tokio::fs::write(&tmp, content).await?;
        if let Err(error) = tokio::fs::rename(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(WorkspaceError::Io(error));
        }
        Ok(())
    }

    async fn resolve(&self, relative: &str) -> Result<PathBuf, WorkspaceError> {
        let relative_path = Path::new(relative);
        if relative.is_empty()
            || relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(WorkspaceError::InvalidPath);
        }
        tokio::fs::create_dir_all(&self.root).await?;
        self.reject_symlink_components(&self.root.join(relative_path))
            .await?;
        Ok(self.root.join(relative_path))
    }

    async fn reject_symlink_components(&self, path: &Path) -> Result<(), WorkspaceError> {
        let mut current = self.root.clone();
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| WorkspaceError::InvalidPath)?;
        for component in relative.components() {
            current.push(component);
            match tokio::fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(WorkspaceError::InvalidPath);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(WorkspaceError::Io(error)),
            }
        }
        Ok(())
    }
}

pub fn router<S>(manager: Arc<WorkspaceManager>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let list_manager = manager.clone();
    let read_manager = manager.clone();
    Router::new()
        .route(
            "/api/v1/workspace/files",
            get(move |headers| list_files(headers, list_manager.clone())),
        )
        .route(
            "/api/v1/workspace/file",
            get(move |headers, query| read_file(headers, query, read_manager.clone()))
                .put(move |headers, body| write_file(headers, body, manager.clone())),
        )
}

async fn list_files(headers: HeaderMap, manager: Arc<WorkspaceManager>) -> Response {
    if let Err(response) = authorize(&headers) {
        return response;
    }
    match manager.list().await {
        Ok(files) => Json(files).into_response(),
        Err(error) => error_response(error),
    }
}

async fn read_file(
    headers: HeaderMap,
    Query(query): Query<FileQuery>,
    manager: Arc<WorkspaceManager>,
) -> Response {
    if let Err(response) = authorize(&headers) {
        return response;
    }
    match manager.read(&query.path).await {
        Ok(document) => Json(document).into_response(),
        Err(error) => error_response(error),
    }
}

async fn write_file(
    headers: HeaderMap,
    Json(request): Json<WriteFileRequest>,
    manager: Arc<WorkspaceManager>,
) -> Response {
    if let Err(response) = authorize(&headers) {
        return response;
    }
    match manager.write(&request.path, &request.content).await {
        Ok(()) => Json(json!({ "saved": true, "path": request.path })).into_response(),
        Err(error) => error_response(error),
    }
}

fn authorize(headers: &HeaderMap) -> Result<(), Response> {
    let expected = std::env::var("GATEWAY_ADMIN_TOKEN")
        .or_else(|_| std::env::var("OPENAB_ADMIN_TOKEN"))
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "admin token is not configured" })),
            )
                .into_response()
        })?;
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let header_token = headers
        .get("x-openab-admin-token")
        .and_then(|value| value.to_str().ok());
    if bearer == Some(expected.as_str()) || header_token == Some(expected.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing admin token" })),
        )
            .into_response())
    }
}

fn map_not_found(error: std::io::Error) -> WorkspaceError {
    if error.kind() == std::io::ErrorKind::NotFound {
        WorkspaceError::NotFound
    } else {
        WorkspaceError::Io(error)
    }
}

fn error_response(error: WorkspaceError) -> Response {
    match error {
        WorkspaceError::InvalidPath => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "workspace path must be a safe relative path" })),
        )
            .into_response(),
        WorkspaceError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "workspace file not found" })),
        )
            .into_response(),
        WorkspaceError::TooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "workspace file exceeds 1 MiB" })),
        )
            .into_response(),
        WorkspaceError::Io(error) => {
            tracing::error!(error = %error, "workspace admin error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_path_traversal() {
        let manager = WorkspaceManager::new(std::env::temp_dir().join("openab-workspace-test"));
        assert!(matches!(
            manager.resolve("../secret").await,
            Err(WorkspaceError::InvalidPath)
        ));
        assert!(matches!(
            manager.resolve("/etc/passwd").await,
            Err(WorkspaceError::InvalidPath)
        ));
    }

    #[tokio::test]
    async fn writes_and_reads_managed_file() {
        let root = std::env::temp_dir().join(format!("openab-workspace-{}", uuid::Uuid::new_v4()));
        let manager = WorkspaceManager::new(&root);
        manager.write("AGENTS.md", "# Instructions").await.unwrap();
        let document = manager.read("AGENTS.md").await.unwrap();
        assert_eq!(document.content, "# Instructions");
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
