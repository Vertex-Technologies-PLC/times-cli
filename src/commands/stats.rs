use chrono::{Duration, NaiveDate, Utc};

use crate::api::types::Stats;
use crate::cli::StatsArgs;
use crate::context::Ctx;
use crate::error::CliError;
use crate::ui::etb;

pub async fn run(ctx: &mut Ctx, args: StatsArgs) -> anyhow::Result<()> {
    let today = Utc::now().date_naive();
    let (from, to) = date_range(&args, today)?;
    let mut query = vec![("from", from.to_string()), ("to", to.to_string())];
    if let Some(slug) = &args.slug {
        query.push(("slug", slug.clone()));
    }
    let token = ctx.erp_token().await?;
    let stats: Stats = ctx
        .client
        .get("/api/v1/developers/stats", Some(&token), &query)
        .await?;
    if ctx.out.json {
        return ctx.out.json(&stats);
    }
    let scope = args.slug.as_deref().unwrap_or("all AIPs");
    ctx.out.info(format!(
        "Stats for {scope} from {} to {}",
        stats.from, stats.to
    ));
    ctx.out.fields(&[
        ("installs", stats.installs.to_string()),
        ("usage events", stats.usage_events.to_string()),
        ("gross", etb(stats.gross_etb)),
        ("your share", etb(stats.publisher_share_etb)),
    ]);
    let active: Vec<_> = stats
        .days
        .iter()
        .filter(|d| d.installs > 0 || d.usage_events > 0 || d.gross_etb > 0.0)
        .collect();
    if !active.is_empty() {
        println!();
        let rows = active
            .iter()
            .map(|d| {
                vec![
                    d.day.clone(),
                    d.installs.to_string(),
                    d.usage_events.to_string(),
                    etb(d.publisher_share_etb),
                ]
            })
            .collect();
        ctx.out
            .table(&["day", "installs", "events", "your share"], rows);
    }
    Ok(())
}

fn parse_date(s: &str) -> anyhow::Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| CliError::usage(format!("`{s}` is not a date. Use YYYY-MM-DD.")))
}

pub fn parse_days(s: &str) -> Option<i64> {
    let n: i64 = s.strip_suffix('d')?.parse().ok()?;
    (1..=3650).contains(&n).then_some(n)
}

fn date_range(args: &StatsArgs, today: NaiveDate) -> anyhow::Result<(NaiveDate, NaiveDate)> {
    let to = match &args.to {
        Some(s) => parse_date(s)?,
        None => today,
    };
    let from = match &args.from {
        Some(s) => parse_date(s)?,
        None => {
            let days = parse_days(&args.range).ok_or_else(|| {
                CliError::usage(format!(
                    "`{}` is not a range. Use a number of days like 7d or 30d.",
                    args.range
                ))
            })?;
            to - Duration::days(days - 1)
        }
    };
    if from > to {
        return Err(CliError::usage("--from must be on or before --to."));
    }
    Ok((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(range: &str, from: Option<&str>, to: Option<&str>) -> StatsArgs {
        StatsArgs {
            slug: None,
            range: range.into(),
            from: from.map(Into::into),
            to: to.map(Into::into),
        }
    }

    #[test]
    fn computes_ranges() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        let (f, t) = date_range(&args("30d", None, None), today).unwrap();
        assert_eq!(f.to_string(), "2026-09-18");
        assert_eq!(t, today);
        let (f, t) =
            date_range(&args("7d", Some("2026-01-01"), Some("2026-01-31")), today).unwrap();
        assert_eq!(
            (f.to_string(), t.to_string()),
            ("2026-01-01".into(), "2026-01-31".into())
        );
        assert!(date_range(&args("week", None, None), today).is_err());
        assert!(date_range(&args("7d", Some("2026-02-01"), Some("2026-01-01")), today).is_err());
    }
}
