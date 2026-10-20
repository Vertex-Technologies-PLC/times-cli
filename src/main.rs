mod api;
mod auth;
mod build;
mod bundle;
mod cli;
mod commands;
mod config;
mod context;
mod error;
mod hosting;
mod manifest;
mod netutil;
mod origin;
mod scan;
mod shell;
mod templates;
mod ui;
mod update;

use clap::Parser;
use console::style;
use serde_json::json;

use cli::{Cli, Command};
use error::{exit, CliError};
use ui::Output;

fn main() {
    if update::is_internal_check() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(update::refresh_from_network());
        return;
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => finish_clap(err),
    };
    let json_mode = cli.global.json;
    let quiet_update =
        json_mode || matches!(cli.command, Command::Completions(_) | Command::Upgrade);
    let result = {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        runtime.block_on(commands::run(cli))
    };
    if let Err(err) = result {
        let code = report(&err, json_mode);
        if !quiet_update {
            warn_about_update();
        }
        std::process::exit(code);
    }
    if !quiet_update {
        warn_about_update();
    }
}

fn finish_clap(err: clap::Error) -> ! {
    let code = err.exit_code();
    let _ = err.print();
    if !std::env::args().any(|arg| arg == "--json") {
        warn_about_update();
    }
    std::process::exit(code);
}

fn warn_about_update() {
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let out = Output::new(false, true, true);
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    runtime.block_on(update::maybe_warn(&out));
}

fn report(err: &anyhow::Error, json_mode: bool) -> i32 {
    let cli_err = err.downcast_ref::<CliError>();
    let code = cli_err.map(|e| e.exit_code()).unwrap_or(exit::GENERAL);
    let hint = cli_err.and_then(|e| e.hint());
    let message = format!("{err:#}");

    if json_mode {
        let mut body = json!({
            "error": {
                "code": cli_err.map(|e| e.code().to_string()).unwrap_or_else(|| "ERROR".into()),
                "message": message,
            }
        });
        if let Some(h) = hint {
            body["error"]["hint"] = json!(h);
        }
        if let Some(CliError::Api {
            status,
            details: Some(d),
            ..
        }) = cli_err
        {
            body["error"]["status"] = json!(status);
            body["error"]["details"] = d.clone();
        } else if let Some(CliError::Api { status, .. }) = cli_err {
            body["error"]["status"] = json!(status);
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        let label = match cli_err {
            Some(CliError::Api { code, .. }) if !code.starts_with("HTTP_") => {
                format!("{} ({code})", style("error:").red().bold())
            }
            _ => style("error:").red().bold().to_string(),
        };
        eprintln!("{label} {message}");
        if let Some(h) = hint {
            eprintln!("{} {h}", style("hint:").cyan());
        }
    }
    code
}
