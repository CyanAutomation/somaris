use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use somaris::api::{AppState, router};
use std::collections::BTreeSet;
use tower::ServiceExt;

#[tokio::test]
async fn dashboard_and_assets_are_served_with_correct_content_types() {
    for (path, content_type, marker) in [
        ("/", "text/html", "rel=\"apple-touch-icon\""),
        ("/ui/app.mjs", "text/javascript", "resolveDisplayMode"),
        (
            "/ui/api-docs.mjs",
            "text/javascript",
            "loadOpenApiReference",
        ),
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

#[tokio::test]
async fn dashboard_module_imports_are_all_served() {
    let app = router(AppState::new());
    let mut pending = vec!["/ui/app.mjs".to_owned()];
    let mut checked = BTreeSet::new();

    while let Some(path) = pending.pop() {
        if !checked.insert(path.clone()) {
            continue;
        }

        let response = app
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/javascript"),
            "{path}"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let source = String::from_utf8(body.to_vec()).expect("UI module is UTF-8");
        pending.extend(module_import_paths(&path, &source));
    }
}

fn module_import_paths(source_path: &str, source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let declaration = line
                .strip_prefix("import ")
                .or_else(|| line.strip_prefix("export "))?;
            let specifier = declaration
                .split_once(" from ")
                .map_or(declaration, |(_, specifier)| specifier)
                .trim();
            let quote = specifier.chars().next()?;
            if quote != '\'' && quote != '"' {
                return None;
            }
            let imported_path = specifier[1..].split(quote).next()?;
            let imported_file = imported_path.strip_prefix("./")?;
            imported_file.ends_with(".mjs").then(|| {
                format!(
                    "{}/{}",
                    source_path.rsplit_once('/').unwrap().0,
                    imported_file
                )
            })
        })
        .collect()
}
