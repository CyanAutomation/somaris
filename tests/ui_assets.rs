use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use somaris::api::{AppState, router};
use tower::ServiceExt;

#[tokio::test]
async fn dashboard_and_assets_are_served_with_correct_content_types() {
    for (path, content_type, marker) in [
        ("/", "text/html", "id=\"display-mode\""),
        ("/ui/app.mjs", "text/javascript", "resolveDisplayMode"),
        ("/ui/style.css", "text/css", "data-display-mode=\"panel\""),
        ("/ui/components.mjs", "text/javascript", "blindState"),
        ("/ui/components.css", "text/css", "icon-action"),
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
        assert!(String::from_utf8_lossy(&body).contains(marker));
    }
}
