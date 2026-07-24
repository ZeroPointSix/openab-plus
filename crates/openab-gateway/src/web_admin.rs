use axum::Router;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;

const INDEX_HTML: &str = include_str!("../../../web/index.html");
const APP_JS: &str = include_str!("../../../web/app.js");
const PROFILES_JS: &str = include_str!("../../../web/profiles.js");
const STYLES_CSS: &str = include_str!("../../../web/styles.css");
const PROFILES_CSS: &str = include_str!("../../../web/profiles.css");

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/", get(admin_redirect))
        .route("/admin", get(admin_index))
        .route("/admin/", get(admin_index))
        .route("/admin/app.js", get(admin_script))
        .route("/admin/profiles.js", get(admin_profiles_script))
        .route("/admin/styles.css", get(admin_styles))
        .route("/admin/profiles.css", get(admin_profiles_styles))
}

async fn admin_redirect() -> Redirect {
    Redirect::temporary("/admin")
}

async fn admin_index() -> Response {
    asset_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn admin_script() -> Response {
    asset_response("application/javascript; charset=utf-8", APP_JS)
}

async fn admin_profiles_script() -> Response {
    asset_response("application/javascript; charset=utf-8", PROFILES_JS)
}

async fn admin_styles() -> Response {
    asset_response("text/css; charset=utf-8", STYLES_CSS)
}

async fn admin_profiles_styles() -> Response {
    asset_response("text/css; charset=utf-8", PROFILES_CSS)
}

fn asset_response(content_type: &'static str, body: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    (headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_index_references_admin_assets() {
        assert!(INDEX_HTML.contains("/admin/app.js"));
        assert!(INDEX_HTML.contains("/admin/profiles.js"));
        assert!(INDEX_HTML.contains("/admin/styles.css"));
        assert!(INDEX_HTML.contains("/admin/profiles.css"));
    }

    #[test]
    fn asset_response_sets_no_cache() {
        let response = asset_response("text/plain; charset=utf-8", "ok");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
    }
}
