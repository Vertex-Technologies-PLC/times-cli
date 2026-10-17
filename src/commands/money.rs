use serde_json::json;

use crate::api::types::{Balance, EarningsPage, Items, PayoutAccount, Withdrawal};
use crate::cli::{EarningsArgs, PayoutCmd};
use crate::context::Ctx;
use crate::error::CliError;
use crate::ui::{etb, or_dash, short_date};

pub async fn earnings(ctx: &mut Ctx, args: EarningsArgs) -> anyhow::Result<()> {
    let token = ctx.erp_token().await?;
    let balance: Balance = ctx
        .client
        .get("/api/v1/developers/balance", Some(&token), &[])
        .await?;
    let mut query = vec![
        ("page", args.page.max(1).to_string()),
        ("pageSize", args.page_size.clamp(1, 100).to_string()),
    ];
    if let Some(slug) = &args.slug {
        query.push(("slug", slug.clone()));
    }
    let page: EarningsPage = ctx
        .client
        .get("/api/v1/developers/earnings", Some(&token), &query)
        .await?;
    if ctx.out.json {
        return ctx
            .out
            .json(&json!({ "balance": balance, "earnings": page }));
    }
    ctx.out.fields(&[
        ("available", etb(balance.available_etb)),
        ("pending", etb(balance.pending_etb)),
        ("reserved", etb(balance.reserved_etb)),
        ("paid out", etb(balance.paid_out_etb)),
        ("lifetime", etb(balance.lifetime_etb)),
    ]);
    if !balance.verified {
        ctx.out
            .warn("Your publisher profile is not verified yet. Payouts start after verification.");
    }
    if page.items.is_empty() {
        ctx.out.info("\nNo earnings yet.");
        return Ok(());
    }
    println!();
    let rows = page
        .items
        .iter()
        .map(|e| {
            vec![
                short_date(e.created_at.as_deref()),
                or_dash(e.slug.clone()),
                etb(e.gross_etb),
                etb(e.publisher_share_etb),
                or_dash(e.status.clone()),
                short_date(e.available_at.as_deref()),
            ]
        })
        .collect();
    ctx.out.table(
        &["date", "aip", "gross", "your share", "status", "available"],
        rows,
    );
    let pages = if page.page_size > 0 {
        (page.total + page.page_size - 1) / page.page_size
    } else {
        1
    };
    if pages > 1 {
        ctx.out.info(format!(
            "Page {} of {}. Use --page to see more.",
            page.page, pages
        ));
    }
    Ok(())
}

pub async fn payout(ctx: &mut Ctx, cmd: PayoutCmd) -> anyhow::Result<()> {
    match cmd {
        PayoutCmd::List => list(ctx).await,
        PayoutCmd::Accounts => accounts(ctx).await,
        PayoutCmd::Request { amount, account } => request(ctx, amount, account).await,
    }
}

async fn fetch_accounts(ctx: &mut Ctx, token: &str) -> anyhow::Result<Vec<PayoutAccount>> {
    let res: Items<PayoutAccount> = ctx
        .client
        .get("/api/v1/developers/payout-accounts", Some(token), &[])
        .await?;
    Ok(res.items)
}

async fn list(ctx: &mut Ctx) -> anyhow::Result<()> {
    let token = ctx.erp_token().await?;
    let res: Items<Withdrawal> = ctx
        .client
        .get("/api/v1/developers/withdrawals", Some(&token), &[])
        .await?;
    if ctx.out.json {
        return ctx.out.json(&res.items);
    }
    if res.items.is_empty() {
        ctx.out
            .info("No withdrawals yet. Request one with `times payout request --amount <ETB>`.");
        return Ok(());
    }
    let rows = res
        .items
        .iter()
        .map(|w| {
            vec![
                short_date(w.requested_at.as_deref()),
                etb(w.amount_etb),
                or_dash(w.payout_account_masked.clone()),
                or_dash(w.status.clone()),
                or_dash(w.reference.clone().or(w.failure_reason.clone())),
            ]
        })
        .collect();
    ctx.out.table(
        &["requested", "amount", "account", "status", "reference"],
        rows,
    );
    Ok(())
}

async fn accounts(ctx: &mut Ctx) -> anyhow::Result<()> {
    let token = ctx.erp_token().await?;
    let items = fetch_accounts(ctx, &token).await?;
    if ctx.out.json {
        return ctx.out.json(&items);
    }
    if items.is_empty() {
        ctx.out
            .info("No payout accounts yet. Add one in the developer portal.");
        return Ok(());
    }
    let rows = items
        .iter()
        .map(|a| {
            vec![
                if a.is_default {
                    "*".into()
                } else {
                    String::new()
                },
                a.id.clone(),
                or_dash(a.method.clone()),
                or_dash(a.bank_name.clone()),
                or_dash(a.account_masked.clone()),
                or_dash(a.status.clone()),
            ]
        })
        .collect();
    ctx.out
        .table(&["", "id", "method", "bank", "account", "status"], rows);
    Ok(())
}

async fn request(ctx: &mut Ctx, amount: f64, account: Option<String>) -> anyhow::Result<()> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(CliError::usage(
            "--amount must be a positive number of ETB.",
        ));
    }
    let amount = (amount * 100.0).round() / 100.0;
    let token = ctx.erp_token().await?;
    let accounts = fetch_accounts(ctx, &token).await?;
    let chosen = match &account {
        Some(id) => accounts.iter().find(|a| &a.id == id).ok_or_else(|| {
            CliError::usage(format!(
                "No payout account with id {id}. Run `times payout accounts`."
            ))
        })?,
        None => accounts.iter().find(|a| a.is_default).ok_or_else(|| {
            CliError::usage(
                "You have no default payout account. Pass --account or add one in the developer portal.",
            )
        })?,
    };
    let label = or_dash(chosen.account_masked.clone().or(chosen.bank_name.clone()));
    if !ctx
        .out
        .confirm(&format!("Withdraw {} to {label}?", etb(amount)))?
    {
        return Err(CliError::usage("Cancelled."));
    }
    let w: Withdrawal = ctx
        .client
        .post(
            "/api/v1/developers/withdrawals",
            Some(&token),
            &json!({ "payoutAccountId": chosen.id, "amountEtb": amount }),
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&w);
    }
    ctx.out.success(format!(
        "Requested {}. Status: {}.",
        etb(w.amount_etb),
        or_dash(w.status.clone())
    ));
    Ok(())
}
