use axum::body::Body;
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use percent_encoding::percent_decode_str;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_CODEG_WEB_ROOT: &str = "/usr/share/openab/codeg";

#[derive(Clone, Debug)]
pub struct Config {
    static_root: Arc<PathBuf>,
}

impl Config {
    pub fn new(static_root: impl Into<PathBuf>) -> Self {
        Self {
            static_root: Arc::new(static_root.into()),
        }
    }

    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("CODEG_WEB_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_CODEG_WEB_ROOT)),
        )
    }
}

pub fn router<S>(config: Config) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/admin", get(retired_admin_redirect))
        .route("/admin/", get(retired_admin_redirect))
        .fallback(get(static_asset))
        .with_state(config)
}

async fn retired_admin_redirect() -> Redirect {
    Redirect::permanent("/")
}

async fn static_asset(State(config): State<Config>, OriginalUri(uri): OriginalUri) -> Response {
    let Some(relative) = safe_relative_path(&uri) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if is_reserved_path(&relative) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let canonical_root = match tokio::fs::canonicalize(config.static_root.as_ref()).await {
        Ok(root) if root.is_dir() => root,
        Ok(_) => {
            tracing::error!("Codeg static root is not a directory");
            return unavailable();
        }
        Err(error) => {
            tracing::error!(%error, "Codeg static root is unavailable");
            return unavailable();
        }
    };

    match find_static_asset(&canonical_root, &relative).await {
        Ok(Some((path, bytes))) => asset_response(&path, bytes),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(%error, "failed to read Codeg static asset");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Codeg workbench assets are unavailable",
    )
        .into_response()
}

fn safe_relative_path(uri: &Uri) -> Option<PathBuf> {
    let decoded = percent_decode_str(uri.path()).decode_utf8().ok()?;
    if decoded.contains('\0') {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in Path::new(decoded.trim_start_matches('/')).components() {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(relative)
}

fn is_reserved_path(relative: &Path) -> bool {
    let Some(Component::Normal(first)) = relative.components().next() else {
        return false;
    };
    matches!(
        first.to_str(),
        Some("api" | "health" | "ws" | "acp" | "webhook")
    )
}

async fn find_static_asset(
    root: &Path,
    relative: &Path,
) -> Result<Option<(PathBuf, Vec<u8>)>, std::io::Error> {
    for candidate in candidate_paths(root, relative) {
        let canonical = match tokio::fs::canonicalize(&candidate).await {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !canonical.starts_with(root) || !tokio::fs::metadata(&canonical).await?.is_file() {
            continue;
        }
        return tokio::fs::read(&canonical)
            .await
            .map(|bytes| Some((canonical, bytes)));
    }
    Ok(None)
}

fn candidate_paths(root: &Path, relative: &Path) -> Vec<PathBuf> {
    if relative.as_os_str().is_empty() {
        return vec![root.join("index.html")];
    }

    let mut candidates = Vec::with_capacity(4);
    candidates.push(root.join(relative));

    if relative.extension().is_none() {
        let mut html_path = OsString::from(relative.as_os_str());
        html_path.push(".html");
        candidates.push(root.join(html_path));
        candidates.push(root.join(relative).join("index.html"));
        candidates.push(root.join("index.html"));
    }
    candidates
}

fn asset_response(path: &Path, bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(path))
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control(path)),
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(bytes))
        .expect("static asset response must be valid")
}

fn cache_control(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|value| value.to_str());
    if matches!(extension, Some("html" | "txt")) {
        "no-cache"
    } else if path
        .components()
        .any(|component| component.as_os_str() == "_next")
    {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    }
}

fn content_type(path: &Path) -> HeaderValue {
    let value = match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    };
    HeaderValue::from_static(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::fs;
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn fixture() -> (TempDir, Config) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("index.html"), "root-index").unwrap();
        fs::write(root.path().join("workspace.html"), "workspace").unwrap();
        fs::write(root.path().join("logo.svg"), "<svg/>").unwrap();
        let chunks = root.path().join("_next/static/chunks");
        fs::create_dir_all(&chunks).unwrap();
        fs::write(chunks.join("app-deadbeef.js"), "chunk").unwrap();
        let config = Config::new(root.path());
        (root, config)
    }

    async fn request(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn serves_export_routes_and_client_fallback() {
        let (_root, config) = fixture();
        let app = router::<()>(config);
        for (uri, expected) in [
            ("/", "root-index"),
            ("/workspace", "workspace"),
            ("/workspace?restored=1", "workspace"),
            ("/future/client-route", "root-index"),
        ] {
            let response = request(app.clone(), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(body_text(response).await, expected, "{uri}");
        }
    }

    #[tokio::test]
    async fn retired_admin_redirects_to_codeg_root() {
        let (_root, config) = fixture();
        for uri in ["/admin", "/admin/"] {
            let response = request(router::<()>(config.clone()), uri).await;
            assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
            assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/");
        }
    }

    #[tokio::test]
    async fn reserved_control_plane_paths_never_fall_back_to_html() {
        let (_root, config) = fixture();
        let app = router::<()>(config);
        for uri in [
            "/api/not-real",
            "/api/v1/missing",
            "/ws/events",
            "/webhook/missing",
        ] {
            let response = request(app.clone(), uri).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn missing_asset_with_extension_is_not_spa_fallback() {
        let (_root, config) = fixture();
        let response = request(router::<()>(config), "/_next/static/missing.js").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rejects_encoded_parent_traversal() {
        let (_root, config) = fixture();
        let response = request(router::<()>(config), "/%2e%2e/secret").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let (_root, config) = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), config.static_root.join("escape")).unwrap();
        let response = request(router::<()>(config), "/escape/secret.txt").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn applies_html_asset_and_hashed_cache_policies() {
        let (_root, config) = fixture();
        let app = router::<()>(config);
        for (uri, expected) in [
            ("/workspace", "no-cache"),
            ("/logo.svg", "public, max-age=3600"),
            (
                "/_next/static/chunks/app-deadbeef.js",
                "public, max-age=31536000, immutable",
            ),
        ] {
            let response = request(app.clone(), uri).await;
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                expected
            );
            assert_eq!(
                response
                    .headers()
                    .get(header::X_CONTENT_TYPE_OPTIONS)
                    .unwrap(),
                "nosniff"
            );
        }
    }

    #[tokio::test]
    async fn reports_missing_bundle_without_blocking_startup() {
        let missing = tempfile::tempdir().unwrap().path().join("not-present");
        let response = request(router::<()>(Config::new(missing)), "/").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body_text(response).await,
            "Codeg workbench assets are unavailable"
        );
    }
}
