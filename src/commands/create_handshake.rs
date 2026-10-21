use std::collections::HashMap;
use std::io::Read;
use std::net::TcpListener;
use std::time::Duration;

use anyhow::Context;
use reqwest::Url;
use tiny_http::{Header, Method, Response, Server};

use crate::bundle::sha256_hex;
use crate::error::CliError;

const DEVELOPERS_ORIGIN: &str = "https://developers.times.et";
const CALLBACK_PATH: &str = "/cli-create";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateHandshake {
    Created(Identity),
    Cancelled,
}

pub struct LocalCreateServer {
    listener: TcpListener,
    pub port: u16,
    pub token: String,
}

impl LocalCreateServer {
    pub fn bind() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("Could not bind a local port for the create handshake")?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port,
            token: new_token(),
        })
    }

    pub fn portal_url(&self, prefill: &Identity) -> anyhow::Result<String> {
        let mut url = Url::parse(&format!("{DEVELOPERS_ORIGIN}/dashboard/aips"))
            .expect("developers origin is valid");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("create", "1");
            q.append_pair("cli", "1");
            q.append_pair("cli_port", &self.port.to_string());
            q.append_pair("cli_token", &self.token);
            if !prefill.name.is_empty() {
                q.append_pair("name", &prefill.name);
            }
            if !prefill.slug.is_empty() {
                q.append_pair("slug", &prefill.slug);
            }
            if !prefill.kind.is_empty() {
                q.append_pair("kind", &prefill.kind);
            }
            if !prefill.mode.is_empty() {
                q.append_pair("mode", &prefill.mode);
            }
        }
        Ok(url.into())
    }

    pub async fn wait(self, timeout: Duration) -> anyhow::Result<CreateHandshake> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let token = self.token;
        let listener = self.listener;
        std::thread::spawn(move || {
            let result = (|| {
                let server = Server::from_listener(listener, None)
                    .map_err(|e| anyhow::anyhow!("Could not start local create server: {e}"))?;
                serve_until_result(&server, &token)
            })();
            let _ = tx.send(result);
        });

        tokio::select! {
            biased;
            result = rx => {
                result.map_err(|_| CliError::usage("Create handshake ended unexpectedly."))?
            }
            _ = tokio::signal::ctrl_c() => {
                Err(CliError::usage("Cancelled."))
            }
            _ = tokio::time::sleep(timeout) => {
                Err(CliError::usage(
                    "Timed out waiting for create in the browser. Run `times aip create` again.",
                ))
            }
        }
    }
}

fn new_token() -> String {
    let mut buf = [0u8; 32];
    fill_random(&mut buf);
    sha256_hex(&buf)
}

fn fill_random(buf: &mut [u8]) {
    #[cfg(unix)]
    {
        if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
            if file.read_exact(buf).is_ok() {
                return;
            }
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = ((nanos >> ((i % 8) * 8)) as u8) ^ ((pid >> ((i % 4) * 8)) as u8) ^ (i as u8);
    }
}

fn serve_until_result(server: &Server, expected_token: &str) -> anyhow::Result<CreateHandshake> {
    loop {
        let request = server.recv().context("Local create server failed")?;
        if request.method() == &Method::Options {
            let response = cors_response(Response::empty(204));
            let _ = request.respond(response);
            continue;
        }

        let raw_url = request.url().to_string();
        let path = raw_url.split('?').next().unwrap_or(raw_url.as_str());
        if path != CALLBACK_PATH {
            let _ = request.respond(Response::from_string("Not found").with_status_code(404));
            continue;
        }

        match parse_callback(&raw_url, expected_token) {
            Ok(result) => {
                let body = match &result {
                    CreateHandshake::Created(id) => result_page(
                        ResultKind::Created,
                        &id.name,
                        &id.slug,
                    ),
                    CreateHandshake::Cancelled => {
                        result_page(ResultKind::Cancelled, "", "")
                    }
                };
                let response = cors_response(
                    Response::from_string(body)
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap()),
                );
                let _ = request.respond(response);
                return Ok(result);
            }
            Err(err) => {
                let msg = err.to_string();
                let response = cors_response(
                    Response::from_string(msg.clone())
                        .with_status_code(400)
                        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"text/plain; charset=utf-8"[..]).unwrap()),
                );
                let _ = request.respond(response);
            }
        }
    }
}

fn cors_response<R: std::io::Read>(response: Response<R>) -> Response<R> {
    response
        .with_header(
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"https://developers.times.et"[..])
                .unwrap(),
        )
        .with_header(
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, OPTIONS"[..]).unwrap(),
        )
        .with_header(
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"*"[..]).unwrap(),
        )
}

pub fn parse_callback(url: &str, expected_token: &str) -> anyhow::Result<CreateHandshake> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params = parse_query(query);
    let token = params
        .get("token")
        .map(String::as_str)
        .unwrap_or("");
    if token.is_empty() || token != expected_token {
        return Err(CliError::usage("Invalid create handshake token.").into());
    }
    let status = params
        .get("status")
        .map(String::as_str)
        .unwrap_or("");
    match status {
        "cancelled" | "canceled" => Ok(CreateHandshake::Cancelled),
        "created" => {
            let slug = required_param(&params, "slug")?;
            let name = params
                .get("name")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| slug.clone());
            let kind = params
                .get("kind")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "app".into());
            let mode = params
                .get("mode")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "platform".into());
            Ok(CreateHandshake::Created(Identity {
                name,
                slug,
                kind,
                mode,
            }))
        }
        _ => Err(CliError::usage("Unknown create handshake status.").into()),
    }
}

fn required_param(params: &HashMap<String, String>, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CliError::usage(format!("Create handshake missing {key}.")).into())
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &input[i + 1..i + 3];
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

enum ResultKind {
    Created,
    Cancelled,
}

fn result_page(kind: ResultKind, name: &str, slug: &str) -> String {
    let (title, kicker, detail, mark_class) = match kind {
        ResultKind::Created => {
            let label = if name.is_empty() {
                html_escape(slug)
            } else {
                html_escape(name)
            };
            let slug_line = if slug.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"slug\"><code>{}</code></p>",
                    html_escape(slug)
                )
            };
            (
                "AIP created",
                "Ready on TIMES",
                format!(
                    "<p class=\"lead\">{label} is set up.</p>{slug_line}<p class=\"hint\">Return to your terminal to keep going. You can close this tab.</p>"
                ),
                "mark-ok",
            )
        }
        ResultKind::Cancelled => (
            "Create cancelled",
            "TIMES CLI",
            "<p class=\"lead\">Nothing was created.</p><p class=\"hint\">Return to your terminal. You can close this tab.</p>"
                .into(),
            "mark-cancel",
        ),
    };

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <meta name="viewport" content="width=device-width, initial-scale=1"/>
  <title>{title} · TIMES</title>
  <link rel="preconnect" href="https://fonts.googleapis.com"/>
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin/>
  <link href="https://fonts.googleapis.com/css2?family=DM+Sans:ital,opsz,wght@0,9..40,500;0,9..40,700;1,9..40,500&family=JetBrains+Mono:wght@500&display=swap" rel="stylesheet"/>
  <style>
    :root {{
      --ink: #12141a;
      --muted: #5c6370;
      --paper: #f3f1eb;
      --panel: rgba(255,255,255,0.72);
      --line: rgba(18,20,26,0.1);
      --ok: #1f7a4c;
      --ok-soft: #d8efe3;
      --cancel: #6b7280;
      --cancel-soft: #e8eaed;
    }}
    * {{ box-sizing: border-box; }}
    html, body {{ height: 100%; margin: 0; }}
    body {{
      font-family: "DM Sans", system-ui, sans-serif;
      color: var(--ink);
      background:
        radial-gradient(1200px 700px at 12% -10%, #dfe8ff 0%, transparent 55%),
        radial-gradient(900px 600px at 100% 0%, #e7f5ec 0%, transparent 50%),
        linear-gradient(180deg, #faf9f6 0%, var(--paper) 100%);
      display: grid;
      place-items: center;
      padding: 2rem 1.25rem;
    }}
    main {{
      width: min(100%, 28rem);
      text-align: center;
    }}
    .brand {{
      font-size: 0.75rem;
      font-weight: 700;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      color: var(--ink);
      margin: 0 0 1.75rem;
    }}
    .panel {{
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 1.25rem;
      padding: 2rem 1.5rem 1.75rem;
      backdrop-filter: blur(10px);
      box-shadow: 0 18px 40px rgba(18, 20, 26, 0.06);
    }}
    .mark {{
      width: 3.25rem;
      height: 3.25rem;
      margin: 0 auto 1.1rem;
      border-radius: 999px;
      display: grid;
      place-items: center;
      font-size: 1.35rem;
      font-weight: 700;
      line-height: 1;
    }}
    .mark-ok {{ background: var(--ok-soft); color: var(--ok); }}
    .mark-cancel {{ background: var(--cancel-soft); color: var(--cancel); }}
    .kicker {{
      margin: 0 0 0.35rem;
      font-size: 0.8rem;
      font-weight: 500;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      color: var(--muted);
    }}
    h1 {{
      margin: 0 0 0.85rem;
      font-size: clamp(1.65rem, 4vw, 2rem);
      font-weight: 700;
      letter-spacing: -0.03em;
      line-height: 1.15;
    }}
    .lead {{
      margin: 0;
      font-size: 1.05rem;
      line-height: 1.45;
      color: var(--ink);
    }}
    .slug {{
      margin: 0.65rem 0 0;
    }}
    .slug code {{
      font-family: "JetBrains Mono", ui-monospace, monospace;
      font-size: 0.82rem;
      font-weight: 500;
      padding: 0.28rem 0.55rem;
      border-radius: 0.45rem;
      background: rgba(18, 20, 26, 0.06);
      color: var(--ink);
    }}
    .hint {{
      margin: 1.15rem 0 0;
      font-size: 0.92rem;
      line-height: 1.45;
      color: var(--muted);
    }}
  </style>
</head>
<body>
  <main>
    <p class="brand">TIMES</p>
    <div class="panel">
      <div class="mark {mark_class}" aria-hidden="true">{mark_glyph}</div>
      <p class="kicker">{kicker}</p>
      <h1>{title}</h1>
      {detail}
    </div>
  </main>
</body>
</html>
"##,
        title = title,
        kicker = kicker,
        detail = detail,
        mark_class = mark_class,
        mark_glyph = match kind {
            ResultKind::Created => "✓",
            ResultKind::Cancelled => "×",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn portal_url_includes_handshake_and_prefill() {
        let server = LocalCreateServer::bind().unwrap();
        let port = server.port.to_string();
        let token = server.token.clone();
        let url = server
            .portal_url(&Identity {
                name: "Invoice Reminders".into(),
                slug: "invoice-reminders".into(),
                kind: "app".into(),
                mode: "platform".into(),
            })
            .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(parsed.path(), "/dashboard/aips");
        let pairs: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("create").map(String::as_str), Some("1"));
        assert_eq!(pairs.get("cli").map(String::as_str), Some("1"));
        assert_eq!(pairs.get("cli_port"), Some(&port));
        assert_eq!(pairs.get("cli_token"), Some(&token));
        assert_eq!(
            pairs.get("slug").map(String::as_str),
            Some("invoice-reminders")
        );
        assert_eq!(pairs.get("kind").map(String::as_str), Some("app"));
    }

    #[test]
    fn parse_callback_created_and_cancelled() {
        let token = "abc123";
        let created = parse_callback(
            "/cli-create?status=created&token=abc123&slug=my-app&name=My%20App&kind=app&mode=runtime",
            token,
        )
        .unwrap();
        assert_eq!(
            created,
            CreateHandshake::Created(Identity {
                name: "My App".into(),
                slug: "my-app".into(),
                kind: "app".into(),
                mode: "runtime".into(),
            })
        );
        assert_eq!(
            parse_callback("/cli-create?status=cancelled&token=abc123", token).unwrap(),
            CreateHandshake::Cancelled
        );
        assert!(parse_callback("/cli-create?status=created&token=wrong&slug=x", token).is_err());
    }

    #[test]
    fn local_server_accepts_created_callback() {
        let server = LocalCreateServer::bind().unwrap();
        let port = server.port;
        let token = server.token.clone();
        let handle = thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(server.wait(Duration::from_secs(5)))
        });

        thread::sleep(Duration::from_millis(50));
        let path = format!(
            "{CALLBACK_PATH}?status=created&token={token}&slug=demo-app&name=Demo&kind=integration&mode=platform"
        );
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut body = String::new();
        stream.read_to_string(&mut body).unwrap();
        assert!(body.contains("AIP created"));

        let result = handle.join().unwrap().unwrap();
        assert_eq!(
            result,
            CreateHandshake::Created(Identity {
                name: "Demo".into(),
                slug: "demo-app".into(),
                kind: "integration".into(),
                mode: "platform".into(),
            })
        );
    }
}
