use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use somaris::api::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn dashboard_and_assets_are_served_with_correct_content_types() {
    for (path, content_type, marker) in [
        ("/", "text/html", "rel=\"apple-touch-icon\""),
        ("/ui/app.mjs", "text/javascript", "resolveDisplayMode"),
        ("/ui/api-docs.mjs", "text/javascript", "loadOpenApiReference"),
        ("/ui/style.css", "text/css", ".app-shell"),
        ("/ui/components.mjs", "text/javascript", "blindState"),
        ("/ui/snackbar.mjs", "text/javascript", "class SnackbarQueue"),
        ("/ui/components.css", "text/css", ".ui-icon"),
        ("/ui/panel.css", "text/css", "card--panel"),
        ("/ui/favicon.svg", "image/svg+xml", "#ff9638"),
        ("/ui/favicon-32x32.png", "image/png", "IHDR"),
        ("/ui/apple-touch-icon.png", "image/png", "IHDR"),
    ] {
        let response = router(AppState::new())
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with(content_type)
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains(marker),
            "{path} did not contain expected marker {marker}"
        );
    }
}
