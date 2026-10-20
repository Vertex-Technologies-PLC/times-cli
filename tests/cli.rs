use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{body_json, body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

struct Env {
    config: TempDir,
    work: TempDir,
    api: String,
}

impl Env {
    fn new(server: &MockServer) -> Self {
        Self {
            config: tempfile::tempdir().unwrap(),
            work: tempfile::tempdir().unwrap(),
            api: server.uri(),
        }
    }

    fn cmd(&self, dir: &Path) -> Command {
        let mut c = Command::cargo_bin("times").unwrap();
        c.current_dir(dir)
            .env("TIMES_API_URL", &self.api)
            .env("TIMES_CONFIG_DIR", self.config.path())
            .env("TIMES_CREDENTIAL_STORE", "file")
            .env("TIMES_NO_INPUT", "1")
            .env("TIMES_NO_UPDATE_CHECK", "1")
            .env_remove("TIMES_TOKEN")
            .env_remove("TIMES_ORG")
            .env_remove("TIMES_PROFILE");
        c
    }

    async fn run(&self, dir: &Path, args: &[&str], token: Option<&str>) -> Output {
        let mut c = self.cmd(dir);
        c.args(args);
        if let Some(t) = token {
            c.env("TIMES_TOKEN", t);
        }
        tokio::task::spawn_blocking(move || c.output().unwrap())
            .await
            .unwrap()
    }
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}). stdout: {} stderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn auth_ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "code": "OK", "message": "ok", "data": data
    }))
}

fn auth_err(status: u16, code: &str, details: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": false, "code": code, "message": code, "details": details
    }))
}

fn erp_ok(status: u16, data: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "internalCode": format!("SUCCESS-{status}"),
        "statusCode": status,
        "status": "SUCCESS",
        "message": "ok",
        "data": data
    }))
}

fn erp_err(status: u16, code: &str, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "internalCode": format!("{code}-{status}"),
        "statusCode": status,
        "status": "ERROR",
        "message": message,
        "data": null
    }))
}

fn org(id: &str, slug: &str) -> Value {
    json!({ "id": id, "name": slug.to_uppercase(), "slug": slug, "role": "owner" })
}

fn session(org_value: Value, orgs: Value) -> Value {
    json!({
        "access_token": "acc-1",
        "access_token_expires_at": "2099-01-01T00:00:00Z",
        "refresh_token": "ref-1",
        "refresh_token_expires_at": "2099-01-01T00:00:00Z",
        "user": { "id": "u1", "email": "dev@example.com", "first_name": "Dev" },
        "org": org_value,
        "orgs": orgs
    })
}

async fn mock_pat(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/developer/tokens/exchange"))
        .and(body_json(json!({ "token": "tdk_test" })))
        .respond_with(auth_ok(json!({
            "access_token": "pat-access",
            "access_token_expires_at": "2099-01-01T00:00:00Z",
            "org_id": "org-1",
            "scopes": ["aips:read", "aips:write", "aips:submit"],
            "token_id": "tok-1"
        })))
        .mount(server)
        .await;
}

fn submission(status: &str) -> Value {
    json!({
        "id": "sub-1", "slug": "demo-app", "version": "1.0.0", "status": status,
        "scanStatus": "passed", "createdAt": "2026-10-17T10:00:00Z"
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn init_without_path_creates_slug_folder() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    let out = env
        .run(
            env.work.path(),
            &[
                "init",
                "--slug",
                "times-peachtree",
                "--kind",
                "integration",
                "--framework",
                "static",
                "--json",
            ],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = stdout_json(&out);
    assert_eq!(v["slug"], "times-peachtree");
    let dir = env.work.path().join("times-peachtree");
    assert!(dir.join("times.aip.json").exists());
    assert!(!env.work.path().join("times.aip.json").exists());

    let out = env
        .run(
            env.work.path(),
            &[
                "init",
                ".",
                "--slug",
                "in-place",
                "--kind",
                "app",
                "--framework",
                "static",
                "--json",
            ],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(env.work.path().join("times.aip.json").exists());
}

async fn scaffold(env: &Env) -> PathBuf {
    let out = env
        .run(env.work.path(), &["init", "demo-app", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dir = env.work.path().join("demo-app");
    let _ = std::fs::remove_dir_all(dir.join("out"));
    let _ = std::fs::remove_dir_all(dir.join("dist"));
    let _ = std::fs::remove_dir_all(dir.join("build"));
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(
        dir.join("dist/index.html"),
        concat!(
            "<!doctype html><html><head><style>",
            "[data-theme=claude]{}[data-theme=discord]{}[data-theme=supabase]{}",
            "[data-theme=vercel]{}[data-theme=mono]{}[data-theme=notebook]{}",
            "[data-theme=light-green]{}[data-theme=zen]{}[data-theme=astro-vista]{}",
            "[data-theme=whatsapp]{}",
            "</style></head><body>",
            "<script>createTimesAipClient();syncPreferencesFromApi()</script>",
            "<p>hi</p></body></html>"
        ),
    )
    .unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn device_login_polls_then_selects_org() {
    let server = MockServer::start().await;
    let env = Env::new(&server);

    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/code"))
        .and(body_json(json!({ "client_name": "times-cli" })))
        .respond_with(auth_ok(json!({
            "device_code": "dev-code",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://developers.times.et/device",
            "interval": 1,
            "expires_in": 60
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(auth_err(400, "authorization_pending", Value::Null))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .and(body_json(json!({ "device_code": "dev-code" })))
        .respond_with(auth_ok(session(
            Value::Null,
            json!([org("org-1", "acme"), org("org-2", "beta")]),
        )))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/developer/orgs/org-2/select"))
        .and(body_json(json!({ "refresh_token": "ref-1" })))
        .and(header("authorization", "Bearer acc-1"))
        .respond_with(auth_ok(session(org("org-2", "beta"), json!([]))))
        .expect(1)
        .mount(&server)
        .await;

    let out = env
        .run(
            env.work.path(),
            &["auth", "login", "--no-browser", "--org", "beta", "--json"],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = stdout_json(&out);
    assert_eq!(v["org"]["slug"], "beta");
    assert_eq!(v["user"]["email"], "dev@example.com");
    assert_eq!(v["orgs"].as_array().unwrap().len(), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("ABCD-EFGH"));

    Mock::given(method("GET"))
        .and(path("/api/v1/auth/developer/me"))
        .and(header("authorization", "Bearer acc-1"))
        .respond_with(auth_ok(json!({
            "user": { "id": "u1", "email": "dev@example.com" },
            "org": org("org-2", "beta"),
            "orgs": [org("org-1", "acme"), org("org-2", "beta")]
        })))
        .mount(&server)
        .await;
    let out = env
        .run(env.work.path(), &["auth", "whoami", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["org"]["slug"], "beta");
}

#[tokio::test(flavor = "multi_thread")]
async fn denied_login_exits_with_auth_code() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/code"))
        .respond_with(auth_ok(json!({
            "device_code": "d", "user_code": "U", "verification_uri": "https://x", "interval": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(auth_err(400, "access_denied", Value::Null))
        .mount(&server)
        .await;
    let out = env
        .run(env.work.path(), &["auth", "login", "--no-browser"], None)
        .await;
    assert_eq!(out.status.code(), Some(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn org_use_resolves_slug_and_selects() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/code"))
        .respond_with(auth_ok(json!({
            "device_code": "d", "user_code": "U", "verification_uri": "https://x", "interval": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(auth_ok(session(
            org("org-1", "acme"),
            json!([org("org-1", "acme"), org("org-2", "beta")]),
        )))
        .mount(&server)
        .await;
    let out = env
        .run(env.work.path(), &["auth", "login", "--no-browser"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    Mock::given(method("GET"))
        .and(path("/api/v1/auth/developer/orgs"))
        .respond_with(auth_ok(json!([org("org-1", "acme"), org("org-2", "beta")])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/developer/orgs/org-2/select"))
        .respond_with(auth_ok(session(org("org-2", "beta"), json!([]))))
        .expect(1)
        .mount(&server)
        .await;

    let out = env
        .run(env.work.path(), &["org", "use", "beta", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["org"]["id"], "org-2");

    let out = env
        .run(env.work.path(), &["org", "use", "nope"], None)
        .await;
    assert!(!out.status.success());
}

fn depot_ok(status: u16, data: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "success": true, "data": data
    }))
}

fn depot_job(status: &str) -> Value {
    json!({
        "id": "job-1",
        "slug": "demo-app",
        "kind": "build_and_stage",
        "status": status,
        "trigger_kind": "cli_local",
        "version": "1.0.0",
        "attempt": 1
    })
}

async fn mock_depot_local_deploy(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/depot/source-upload"))
        .and(header("authorization", "Bearer pat-access"))
        .and(header("content-type", "application/zip"))
        .respond_with(depot_ok(
            200,
            json!({
                "storage_key": "cas/abc",
                "sha256": "deadbeef",
                "deduped": false
            }),
        ))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/depot/jobs"))
        .and(header("authorization", "Bearer pat-access"))
        .and(body_partial_json(json!({
            "slug": "demo-app",
            "kind": "build_and_stage",
            "trigger_kind": "cli_local"
        })))
        .respond_with(depot_ok(202, depot_job("queued")))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deploy_bump_updates_manifest_and_package_json() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    mock_depot_local_deploy(&server).await;
    let dir = scaffold(&env).await;
    std::fs::write(
        dir.join("package.json"),
        r#"{"name":"demo-app","version":"1.0.0"}"#,
    )
    .unwrap();

    let out = env
        .run(
            &dir,
            &["aip", "deploy", "--bump", "patch", "--no-follow", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "code={:?} stdout={} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("times.aip.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["version"], "1.0.1");
    let pkg: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    assert_eq!(pkg["version"], "1.0.1");
}

#[tokio::test(flavor = "multi_thread")]
async fn deploy_uploads_source_and_queues_job() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    mock_depot_local_deploy(&server).await;
    let dir = scaffold(&env).await;

    let out = env
        .run(
            &dir,
            &["aip", "deploy", "--no-follow", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "code={:?} stdout={} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let body = stdout_json(&out);
    assert_eq!(body["id"], "job-1");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["slug"], "demo-app");

    let requests = server.received_requests().await.unwrap();
    let upload: &Request = requests
        .iter()
        .find(|r| {
            r.method.as_str() == "POST" && r.url.path() == "/api/v1/depot/source-upload"
        })
        .unwrap();
    assert_eq!(&upload.body[..2], b"PK");
}

#[tokio::test(flavor = "multi_thread")]
async fn deploy_follow_submit_after_success() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    mock_depot_local_deploy(&server).await;
    let dir = scaffold(&env).await;
    let base = "/api/v1/developers/aips/demo-app";

    Mock::given(method("GET"))
        .and(path("/api/v1/depot/jobs/job-1/logs"))
        .respond_with(depot_ok(200, json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/depot/jobs/job-1"))
        .respond_with(depot_ok(200, depot_job("succeeded")))
        .expect(1..)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{base}/versions/1.0.0/submit")))
        .respond_with(erp_ok(200, submission("in_review")))
        .expect(1)
        .mount(&server)
        .await;

    let out = env
        .run(
            &dir,
            &["aip", "deploy", "--submit", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["status"], "in_review");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_errors_map_to_exit_codes_and_hints() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/developers/aips"))
        .respond_with(erp_err(
            403,
            "INSUFFICIENT_SCOPE",
            "This token needs the aips:read scope",
        ))
        .mount(&server)
        .await;

    let out = env
        .run(env.work.path(), &["aip", "list"], Some("tdk_test"))
        .await;
    assert_eq!(out.status.code(), Some(4));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("aips:read"), "{err}");

    let out = env
        .run(
            env.work.path(),
            &["aip", "list", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert_eq!(out.status.code(), Some(4));
    let v: Value = serde_json::from_slice(&out.stderr)
        .or_else(|_| serde_json::from_slice(&out.stdout))
        .unwrap();
    assert_eq!(v["error"]["code"], "INSUFFICIENT_SCOPE");

    let out = env.run(env.work.path(), &["aip", "list"], None).await;
    assert_eq!(out.status.code(), Some(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_and_stats_print_json() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/developers/aips"))
        .respond_with(erp_ok(
            200,
            json!({ "items": [{ "slug": "demo-app", "kind": "app", "status": "draft", "latestVersion": "1.0.0" }] }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/developers/stats"))
        .respond_with(erp_ok(
            200,
            json!({ "from": "2026-10-11", "to": "2026-10-17", "installs": 3, "usageEvents": 10,
                    "grossEtb": 100.0, "publisherShareEtb": 70.0, "days": [] }),
        ))
        .mount(&server)
        .await;

    let out = env
        .run(
            env.work.path(),
            &["aip", "list", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)[0]["latestVersion"], "1.0.0");

    let out = env
        .run(
            env.work.path(),
            &["stats", "--range", "7d", "--json"],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["installs"], 3);
    let req = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == "/api/v1/developers/stats")
        .unwrap();
    let q = req.url.query().unwrap_or_default().to_string();
    assert!(q.contains("from=") && q.contains("to="), "{q}");
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_shell_print_disables_builtin() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    let out = env
        .run(env.work.path(), &["setup-shell", "--print"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("disable times"), "{stdout}");
    assert!(stdout.contains("enable -n times"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread")]
async fn init_and_validate_work_offline() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    let dir = scaffold(&env).await;

    let out = env
        .run(&dir, &["manifest", "validate", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["valid"], true);

    let manifest = dir.join("times.aip.json");
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    m["slug"] = json!("Bad Slug");
    std::fs::write(&manifest, serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let out = env.run(&dir, &["manifest", "validate"], None).await;
    assert_eq!(out.status.code(), Some(5));

    let out = env.run(&dir, &["aip", "deploy"], Some("tdk_test")).await;
    assert_eq!(out.status.code(), Some(5));

    for locale in ["en", "am", "om", "ti"] {
        assert!(dir.join(format!("i18n/messages/{locale}.json")).exists());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

async fn mock_device_login(server: &MockServer, org_value: Value, orgs: Value) {
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/code"))
        .respond_with(auth_ok(json!({
            "device_code": "d", "user_code": "U", "verification_uri": "https://x", "interval": 1
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/token"))
        .respond_with(auth_ok(session(org_value, orgs)))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_switch_and_refresh() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_device_login(
        &server,
        org("org-2", "beta"),
        json!([org("org-1", "acme"), org("org-2", "beta")]),
    )
    .await;
    mock_pat(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/developer/me"))
        .and(header("authorization", "Bearer acc-1"))
        .respond_with(auth_ok(json!({
            "user": { "id": "u1", "email": "dev@example.com" },
            "org": org("org-2", "beta"),
            "orgs": [org("org-1", "acme"), org("org-2", "beta")]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/developer/refresh"))
        .and(body_partial_json(json!({ "refresh_token": "ref-1" })))
        .respond_with(auth_ok(json!({
            "access_token": "acc-2",
            "access_token_expires_at": "2099-01-01T00:00:00Z",
            "refresh_token": "ref-2",
            "refresh_token_expires_at": "2099-01-01T00:00:00Z",
            "user": { "id": "u1", "email": "dev@example.com", "first_name": "Dev" },
            "org": org("org-2", "beta"),
            "orgs": [org("org-1", "acme"), org("org-2", "beta")]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let out = env
        .run(
            env.work.path(),
            &[
                "auth",
                "login",
                "--no-browser",
                "--profile",
                "work",
                "--json",
            ],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "work");
    assert_eq!(stdout_json(&out)["org"]["slug"], "beta");

    let out = env
        .run(env.work.path(), &["auth", "whoami", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "work");
    assert_eq!(stdout_json(&out)["org"]["slug"], "beta");

    let out = env
        .run(
            env.work.path(),
            &[
                "auth",
                "login",
                "--token",
                "tdk_test",
                "--profile",
                "ci",
                "--json",
            ],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "ci");

    let out = env
        .run(env.work.path(), &["auth", "list", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = stdout_json(&out);
    assert_eq!(listed["current"], "ci");
    let names: Vec<&str> = listed["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"work"));
    assert!(names.contains(&"ci"));

    let out = env
        .run(
            env.work.path(),
            &["--profile", "work", "auth", "whoami", "--json"],
            None,
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "work");
    assert_eq!(stdout_json(&out)["via_token"], false);

    let out = env
        .run(env.work.path(), &["auth", "whoami", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "ci");
    assert_eq!(stdout_json(&out)["via_token"], true);

    let out = env
        .run(env.work.path(), &["auth", "use", "work", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "work");
    assert_eq!(stdout_json(&out)["signed_in"], true);

    let out = env
        .run(env.work.path(), &["auth", "refresh", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["profile"], "work");
    assert_eq!(stdout_json(&out)["org"]["slug"], "beta");

    let out = env
        .run(
            env.work.path(),
            &["--profile", "ci", "auth", "refresh"],
            None,
        )
        .await;
    assert_eq!(out.status.code(), Some(2));

    let out = env
        .run(env.work.path(), &["auth", "delete", "ci", "--json"], None)
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["deleted"], "ci");
    assert_eq!(stdout_json(&out)["current"], "work");

    let out = env
        .run(
            env.work.path(),
            &["--profile", "has space", "auth", "list"],
            None,
        )
        .await;
    assert_eq!(out.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn deploy_remote_queues_clone_job() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    mock_pat(&server).await;
    let dir = scaffold(&env).await;

    Mock::given(method("POST"))
        .and(path("/api/v1/depot/jobs"))
        .and(header("authorization", "Bearer pat-access"))
        .and(body_partial_json(json!({
            "slug": "demo-app",
            "kind": "build_and_stage",
            "trigger_kind": "cli_remote",
            "github_repo_url": "https://github.com/acme/demo-app",
            "git_ref": "main"
        })))
        .respond_with(depot_ok(
            202,
            json!({
                "id": "job-remote",
                "slug": "demo-app",
                "kind": "build_and_stage",
                "status": "queued",
                "trigger_kind": "cli_remote",
                "git_ref": "main",
                "attempt": 1
            }),
        ))
        .expect(1)
        .mount(&server)
        .await;

    let out = env
        .run(
            &dir,
            &[
                "aip",
                "deploy",
                "--remote",
                "https://github.com/acme/demo-app",
                "--no-follow",
                "--json",
            ],
            Some("tdk_test"),
        )
        .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout_json(&out)["id"], "job-remote");
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_fails_closed_on_eval() {
    let server = MockServer::start().await;
    let env = Env::new(&server);
    let dir = scaffold(&env).await;
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/evil.ts"), "eval('alert(1)');\n").unwrap();
    let out = env.run(&dir, &["aip", "scan"], None).await;
    assert_eq!(out.status.code(), Some(5));
    let err = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(err.contains("EVAL") || err.contains("eval("), "{err}");
}
