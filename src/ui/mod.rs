use std::io::IsTerminal;
use std::time::Duration;

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;

use crate::error::CliError;

#[derive(Debug, Clone, Copy)]
pub struct Output {
    pub json: bool,
    pub interactive: bool,
    pub assume_yes: bool,
}

impl Output {
    pub fn new(json: bool, no_input: bool, assume_yes: bool) -> Self {
        let tty = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
        Self {
            json,
            interactive: tty && !no_input && !json,
            assume_yes,
        }
    }

    pub fn json<T: Serialize>(&self, value: &T) -> anyhow::Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }

    pub fn success(&self, msg: impl AsRef<str>) {
        if !self.json {
            eprintln!("{} {}", style("✓").green().bold(), msg.as_ref());
        }
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        if !self.json {
            eprintln!("{}", msg.as_ref());
        }
    }

    pub fn warn(&self, msg: impl AsRef<str>) {
        eprintln!("{} {}", style("warning:").yellow().bold(), msg.as_ref());
    }

    pub fn confirm(&self, prompt: &str) -> anyhow::Result<bool> {
        if self.assume_yes {
            return Ok(true);
        }
        if !self.interactive {
            return Err(CliError::usage(format!(
                "{prompt} Pass --yes to confirm when running without a terminal."
            )));
        }
        inquire::Confirm::new(prompt)
            .with_default(false)
            .prompt()
            .map_err(prompt_error)
    }

    pub fn select(&self, prompt: &str, options: Vec<String>) -> anyhow::Result<usize> {
        let opts: Vec<String> = options.clone();
        let picked = inquire::Select::new(prompt, opts)
            .prompt()
            .map_err(prompt_error)?;
        Ok(options.iter().position(|o| *o == picked).unwrap_or(0))
    }

    pub fn text(&self, prompt: &str, default: &str) -> anyhow::Result<String> {
        inquire::Text::new(prompt)
            .with_default(default)
            .prompt()
            .map_err(prompt_error)
    }

    pub fn progress(&self, len: u64) -> ProgressBar {
        if !self.interactive || len == 0 {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new(len);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {bar:32.cyan/blue} {pos:>2}/{len:<2} {msg}",
            )
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏░"),
        );
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }

    /// Progress bar where `pos` is treated as a percentage (0-100).
    pub fn percent_bar(&self, msg: impl Into<String>) -> ProgressBar {
        if !self.interactive {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new(100);
        pb.set_style(
            ProgressStyle::with_template("{bar:32.cyan/blue} {pos:>3}% {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏░"),
        );
        pb.set_message(msg.into());
        pb
    }

    pub fn spinner(&self, msg: impl Into<String>) -> ProgressBar {
        if !self.interactive {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::with_template("{spinner:.cyan} {msg}").unwrap());
        pb.set_message(msg.into());
        pb.enable_steady_tick(Duration::from_millis(90));
        pb
    }

    pub fn bytes_bar(&self, total: u64, msg: impl Into<String>) -> ProgressBar {
        if !self.interactive {
            return ProgressBar::hidden();
        }
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "{msg} [{bar:30.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec}",
            )
            .unwrap()
            .progress_chars("=> "),
        );
        pb.set_message(msg.into());
        pb
    }

    pub fn step(&self, n: usize, total: usize, msg: impl AsRef<str>) {
        if self.json {
            return;
        }
        eprintln!(
            "{} {}",
            style(format!("[{n}/{total}]")).cyan().bold(),
            msg.as_ref()
        );
    }

    pub fn log_line(&self, stream: Option<&str>, text: &str) {
        if self.json || text.is_empty() {
            return;
        }
        match stream {
            Some("stderr") => eprintln!("{} {}", style("│").red(), text),
            Some("stdout") => eprintln!("{} {}", style("│").dim(), text),
            Some("system") => eprintln!("{} {}", style("▸").cyan().bold(), text),
            _ => eprintln!("{} {}", style("│").dim(), text),
        }
    }

    pub fn status_line(&self, status: &str) {
        if self.json {
            return;
        }
        let label = status.replace('_', " ");
        eprintln!("{} {}", style("●").cyan(), style(label).bold());
    }

    pub fn table(&self, headers: &[&str], rows: Vec<Vec<String>>) {
        if self.json {
            return;
        }
        print!("{}", render_table(headers, &rows));
    }

    pub fn fields(&self, pairs: &[(&str, String)]) {
        if self.json {
            return;
        }
        let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (k, v) in pairs {
            println!("{:<width$}  {}", style(k).bold(), v, width = width);
        }
    }
}

fn prompt_error(e: inquire::InquireError) -> anyhow::Error {
    match e {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            CliError::usage("Cancelled.")
        }
        other => anyhow::anyhow!(other),
    }
}

pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }
    let mut out = String::new();
    let fmt_row = |cells: Vec<String>| -> String {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                line.push_str(cell);
            } else {
                let pad = widths[i].saturating_sub(cell.chars().count());
                line.push_str(cell);
                line.push_str(&" ".repeat(pad + 2));
            }
        }
        line.trim_end().to_string() + "\n"
    };
    out.push_str(&fmt_row(headers.iter().map(|h| h.to_uppercase()).collect()));
    for row in rows {
        out.push_str(&fmt_row(row.clone()));
    }
    out
}

pub fn or_dash(v: Option<impl ToString>) -> String {
    v.map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "-".into())
}

pub fn etb(v: f64) -> String {
    format!("{v:.2} ETB")
}

pub fn short_date(v: Option<&str>) -> String {
    match v {
        Some(s) if s.len() >= 10 => s[..10].to_string(),
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "-".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_columns() {
        let out = render_table(
            &["slug", "status"],
            &[
                vec!["a".into(), "draft".into()],
                vec!["longer-slug".into(), "live".into()],
            ],
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "SLUG         STATUS");
        assert_eq!(lines[1], "a            draft");
        assert_eq!(lines[2], "longer-slug  live");
    }

    #[test]
    fn helpers_format_values() {
        assert_eq!(or_dash(None::<String>), "-");
        assert_eq!(or_dash(Some("")), "-");
        assert_eq!(etb(12.5), "12.50 ETB");
        assert_eq!(short_date(Some("2026-10-17T10:00:00Z")), "2026-10-17");
    }
}
