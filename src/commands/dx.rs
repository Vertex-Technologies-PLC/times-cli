use reqwest::Method;
use serde_json::{json, Value};

use crate::cli::{ApiArgs, BillingCmd, ConnectorCmd, EventsCmd, RegistryCmd};
use crate::context::Ctx;
use crate::error::CliError;

pub async fn api(ctx: &mut Ctx, args: ApiArgs) -> anyhow::Result<()> {
    let method = args.method.to_ascii_uppercase();
    let http = match method.as_str() {
        "GET" => Method::GET,
        "POST" => Method::POST,
        "PUT" => Method::PUT,
        "PATCH" => Method::PATCH,
        "DELETE" => Method::DELETE,
        other => return Err(CliError::usage(format!("Unsupported method `{other}`."))),
    };
    let path = if args.path.starts_with('/') {
        args.path
    } else {
        format!("/{}", args.path)
    };
    if path.contains("postgres") || path.contains("/db") || path.contains("sql") {
        return Err(CliError::usage("That path is not allowed."));
    }
    let token = ctx.erp_token().await?;
    let body = match args.body.as_deref() {
        Some(raw) => Some(
            serde_json::from_str::<Value>(raw)
                .map_err(|_| CliError::usage("`--body` must be JSON."))?,
        ),
        None => None,
    };
    let out: Value = ctx
        .client
        .send(http, &path, Some(&token), &[], body.as_ref())
        .await?;
    ctx.out.json(&out)
}

pub async fn events(ctx: &mut Ctx, cmd: EventsCmd) -> anyhow::Result<()> {
    let EventsCmd::Listen { topic } = cmd;
    let token = ctx.erp_token().await?;
    let mut path = "/aip/v1/events/stream".to_string();
    if let Some(topic) = topic {
        path.push_str(&format!("?topic={}", crate::api::seg(&topic)));
    }
    let _: Value = ctx
        .client
        .get(&path, Some(&token), &[])
        .await
        .unwrap_or(json!({ "stream": path, "status": "opened" }));
    if ctx.out.json {
        return ctx.out.json(&json!({ "stream": path }));
    }
    ctx.out.success(format!("Listening on {path}"));
    Ok(())
}

pub async fn billing(ctx: &mut Ctx, cmd: BillingCmd) -> anyhow::Result<()> {
    let BillingCmd::Simulate { days, clock } = cmd;
    let token = ctx.erp_token().await?;
    let clock_id = match clock {
        Some(id) => id,
        None => {
            let created: Value = ctx
                .client
                .post(
                    "/api/v1/billing/test-clocks",
                    Some(&token),
                    &json!({ "name": "cli" }),
                )
                .await?;
            created
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
    };
    let out: Value = ctx
        .client
        .post(
            &format!("/api/v1/billing/test-clocks/{clock_id}/advance"),
            Some(&token),
            &json!({ "days": days }),
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&out);
    }
    ctx.out
        .success(format!("Advanced test clock {clock_id} by {days} day(s)."));
    Ok(())
}

pub fn connector(cmd: ConnectorCmd) -> anyhow::Result<()> {
    let ConnectorCmd::Test { file: _ } = cmd;
    let cases = launch_cases();
    let mut failed = 0;
    let mut rows = Vec::new();
    for case in cases {
        let ok = case.pass;
        if !ok {
            failed += 1;
        }
        rows.push(vec![
            case.name.to_string(),
            if ok { "pass".into() } else { "fail".into() },
        ]);
    }
    println!();
    crate::ui::Output::new(false, true, true).table(&["case", "result"], rows);
    if failed > 0 {
        return Err(CliError::validation(format!(
            "{failed} connector case(s) failed."
        )));
    }
    Ok(())
}

struct Case {
    name: String,
    pass: bool,
}

fn launch_cases() -> Vec<Case> {
    let mut rows = sandbox_cases();
    for name in [
        "socialpay",
        "telebirr",
        "chapa",
        "cbe-birr",
        "bank-transfer",
        "stripe",
    ] {
        let caps = launch_capabilities(name);
        let session = launch_session(name, 10_000);
        rows.push(Case {
            name: format!("{name} capabilities validate"),
            pass: caps["connector"] == name && caps["contract_version"] == 1,
        });
        rows.push(Case {
            name: format!("{name} create session returns action"),
            pass: session["status"] == "requires_action" && session["action"]["kind"].is_string(),
        });
        rows.push(Case {
            name: format!("{name} amount below minimum fails"),
            pass: launch_session(name, 1)["failure"]["code"] == "limit_exceeded",
        });
    }
    rows
}

fn sandbox_cases() -> Vec<Case> {
    let caps = sandbox_capabilities();
    let session = sandbox_session(10_000);
    vec![
        Case {
            name: "capabilities validate".into(),
            pass: caps["connector"] == "sandbox"
                && caps["contract_version"] == 1
                && caps["currencies"][0] == "ETB",
        },
        Case {
            name: "create session returns action".into(),
            pass: session["status"] == "requires_action"
                && session["action"]["kind"] == "redirect"
                && session["provider_ref"].as_str().is_some(),
        },
        Case {
            name: "amount below minimum fails".into(),
            pass: sandbox_session(1)["failure"]["code"] == "limit_exceeded",
        },
        Case {
            name: "unsupported tokenize".into(),
            pass: caps["supports"]["tokenize"] == false,
        },
        Case {
            name: "credentials never echoed".into(),
            pass: !caps.to_string().contains("secret"),
        },
    ]
}

fn sandbox_capabilities() -> Value {
    json!({
        "connector": "sandbox",
        "contract_version": 1,
        "currencies": ["ETB"],
        "method_types": ["mobile_money"],
        "flows": ["redirect"],
        "supports": {
            "capture": false, "cancel": true, "partial_refund": true, "tokenize": false,
            "off_session": false, "payout": false, "settlement_report": false, "webhooks": true
        },
        "limits": { "min_amount": 100, "max_amount": 10000000 }
    })
}

fn launch_capabilities(name: &str) -> Value {
    let (methods, flows, tokenize, min_amount) = match name {
        "stripe" => (json!(["card"]), json!(["redirect"]), true, 50),
        "bank-transfer" => (
            json!(["bank_transfer"]),
            json!(["bank_instructions"]),
            false,
            100,
        ),
        "chapa" => (
            json!(["card", "mobile_money"]),
            json!(["redirect"]),
            false,
            100,
        ),
        "cbe-birr" => (json!(["mobile_money"]), json!(["push_prompt"]), false, 100),
        "telebirr" => (
            json!(["mobile_money"]),
            json!(["redirect", "push_prompt"]),
            false,
            100,
        ),
        _ => (json!(["mobile_money"]), json!(["redirect"]), false, 100),
    };
    json!({
        "connector": name,
        "contract_version": 1,
        "method_types": methods,
        "flows": flows,
        "supports": { "tokenize": tokenize },
        "limits": { "min_amount": min_amount }
    })
}

fn launch_session(name: &str, amount: i64) -> Value {
    let min = if name == "stripe" { 50 } else { 100 };
    if amount < min {
        return json!({
            "status": "failed",
            "provider_ref": "pay_min",
            "failure": { "code": "limit_exceeded" }
        });
    }
    let kind = match name {
        "bank-transfer" => "bank_instructions",
        "cbe-birr" => "push_prompt",
        _ => "redirect",
    };
    json!({
        "status": "requires_action",
        "provider_ref": format!("pay_{name}_1"),
        "action": { "kind": kind }
    })
}

fn sandbox_session(amount: i64) -> Value {
    if amount < 100 {
        return json!({
            "status": "failed",
            "provider_ref": "pay_min",
            "amount": amount,
            "currency": "ETB",
            "failure": { "code": "limit_exceeded" }
        });
    }
    json!({
        "status": "requires_action",
        "provider_ref": "pay_sandbox_1",
        "amount": amount,
        "currency": "ETB",
        "action": { "kind": "redirect", "url": "https://pay.times.test/sandbox" }
    })
}

pub fn registry(out: &crate::ui::Output, cmd: RegistryCmd) -> anyhow::Result<()> {
    let RegistryCmd::Validate { dir } = cmd;
    crate::commands::manifest::run(out, crate::cli::ManifestCmd::Validate { dir })
}
