use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_expires")]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

fn default_expires() -> u64 {
    600
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DevUser {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
}

impl DevUser {
    pub fn display(&self) -> String {
        let name = [self.first_name.as_deref(), self.last_name.as_deref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let ident = self
            .email
            .clone()
            .or_else(|| self.username.clone())
            .unwrap_or_else(|| self.id.clone());
        if name.is_empty() {
            ident
        } else {
            format!("{name} <{ident}>")
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DevOrg {
    pub id: String,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DevSession {
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token: String,
    pub refresh_token_expires_at: DateTime<Utc>,
    #[serde(default)]
    pub user: DevUser,
    #[serde(default)]
    pub org: Option<DevOrg>,
    #[serde(default)]
    pub orgs: Vec<DevOrg>,
    #[serde(default)]
    pub requires_org: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DevMe {
    #[serde(default)]
    pub user: DevUser,
    #[serde(default)]
    pub org: Option<DevOrg>,
    #[serde(default)]
    pub orgs: Vec<DevOrg>,
    #[serde(default)]
    pub requires_org: bool,
    #[serde(default)]
    pub permissions: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PatExchange {
    pub access_token: String,
    pub access_token_expires_at: DateTime<Utc>,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub token_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub token_prefix: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreatedToken {
    pub id: String,
    pub name: String,
    pub token: String,
    #[serde(default)]
    pub token_prefix: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Items<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Aip {
    pub slug: String,
    pub kind: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub published_version: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub active_installs: Option<i64>,
    #[serde(default)]
    pub total_installs: Option<i64>,
    #[serde(default)]
    pub rating_average: Option<f64>,
    #[serde(default)]
    pub rating_count: Option<i64>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submission {
    #[serde(default)]
    pub id: Option<String>,
    pub slug: String,
    pub version: String,
    pub status: String,
    #[serde(default)]
    pub scan_status: Option<String>,
    #[serde(default)]
    pub scan_notes: Option<String>,
    #[serde(default)]
    pub review_notes: Option<String>,
    #[serde(default)]
    pub bundle_sha256: Option<String>,
    #[serde(default)]
    pub bundle_size_bytes: Option<i64>,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub cdn_url: Option<String>,
    #[serde(default)]
    pub origin_verified_at: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<String>,
    #[serde(default)]
    pub reviewed_at: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionEvent {
    #[serde(default)]
    pub actor_kind: Option<String>,
    #[serde(default)]
    pub from_status: Option<String>,
    #[serde(default)]
    pub to_status: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionDetail {
    pub submission: Submission,
    #[serde(default)]
    pub events: Vec<SubmissionEvent>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub pricing: String,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub support_url: Option<String>,
    #[serde(default)]
    pub privacy_url: Option<String>,
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub price_etb: Option<f64>,
    #[serde(default)]
    pub billing_period: Option<String>,
    #[serde(default)]
    pub trial_days: Option<i64>,
    #[serde(default)]
    pub member_slugs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DepotJob {
    pub id: String,
    pub slug: String,
    #[serde(default)]
    pub kind: Option<String>,
    pub status: String,
    #[serde(default)]
    pub trigger_kind: Option<String>,
    #[serde(default)]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub git_commit_sha: Option<String>,
    #[serde(default)]
    pub source_blob_sha: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub attempt: Option<i32>,
    #[serde(default)]
    pub steps: Vec<DepotJobStep>,
    #[serde(default)]
    pub progress: Option<DepotJobProgress>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DepotJobStep {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DepotJobProgress {
    #[serde(default)]
    pub current: i32,
    #[serde(default)]
    pub total: i32,
    #[serde(default)]
    pub percent: i32,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DepotJobLog {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub line: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsDay {
    pub day: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub installs: i64,
    #[serde(default)]
    pub usage_events: i64,
    #[serde(default)]
    pub gross_etb: f64,
    #[serde(default)]
    pub publisher_share_etb: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub installs: i64,
    #[serde(default)]
    pub usage_events: i64,
    #[serde(default)]
    pub gross_etb: f64,
    #[serde(default)]
    pub publisher_share_etb: f64,
    #[serde(default)]
    pub days: Vec<StatsDay>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    #[serde(default)]
    pub pending_etb: f64,
    #[serde(default)]
    pub available_etb: f64,
    #[serde(default)]
    pub reserved_etb: f64,
    #[serde(default)]
    pub paid_out_etb: f64,
    #[serde(default)]
    pub lifetime_etb: f64,
    #[serde(default)]
    pub min_withdrawal_etb: f64,
    #[serde(default)]
    pub hold_days: i64,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Earning {
    pub id: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub gross_etb: f64,
    #[serde(default)]
    pub publisher_share_etb: f64,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub available_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EarningsPage {
    pub items: Vec<Earning>,
    #[serde(default)]
    pub page: i64,
    #[serde(default)]
    pub page_size: i64,
    #[serde(default)]
    pub total: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayoutAccount {
    pub id: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub bank_name: Option<String>,
    #[serde(default)]
    pub account_name: Option<String>,
    #[serde(default)]
    pub account_masked: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Withdrawal {
    pub id: String,
    #[serde(default)]
    pub payout_account_masked: Option<String>,
    #[serde(default)]
    pub amount_etb: f64,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub requested_at: Option<String>,
    #[serde(default)]
    pub paid_at: Option<String>,
}
