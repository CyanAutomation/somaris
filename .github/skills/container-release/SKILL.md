---
name: container-release
description: Use when changing Somaris Docker packaging, Compose deployment, image tags, GitHub Actions release workflow, or image security checks.
---

# Container and release changes

- Read `Dockerfile`, `.dockerignore`, `docker-compose.yaml`, README deployment instructions, and `.github/workflows/publish-image.yml` together; keep documented runtime paths and ports aligned.
- Preserve pull request validation without publishing. Publishing occurs from eligible `main` pushes only after the required checks and image vulnerability validation succeed.
- Preserve least-privilege workflow permissions, pinned third-party action revisions, credential-free checkout, and Docker Hub credentials through Actions secrets/variables. Never expose credentials to untrusted pull request code.
- Keep architecture declarations consistent across build, scan, publish, and documentation. Image-input path filtering must include every file that can affect the image.
- Validate workflow edits with actionlint (already run in CI); for Dockerfile or dependency changes, use the relevant CI build and vulnerability scan. Do not publish an image manually unless the user explicitly requests it.
