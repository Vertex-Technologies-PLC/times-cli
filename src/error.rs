use serde_json::Value;

pub mod exit {
    pub const GENERAL: i32 = 1;
    pub const USAGE: i32 = 2;
    pub const AUTH: i32 = 3;
    pub const API: i32 = 4;
    pub const VALIDATION: i32 = 5;
    pub const NETWORK: i32 = 6;
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Auth(String),
    #[error("{message}")]
    Api {
        status: u16,
        code: String,
        message: String,
        details: Option<Value>,
    },
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Network(String),
}

impl CliError {
    pub fn usage(msg: impl Into<String>) -> anyhow::Error {
        CliError::Usage(msg.into()).into()
    }

    pub fn auth(msg: impl Into<String>) -> anyhow::Error {
        CliError::Auth(msg.into()).into()
    }

    pub fn validation(msg: impl Into<String>) -> anyhow::Error {
        CliError::Validation(msg.into()).into()
    }

    pub fn network(msg: impl Into<String>) -> anyhow::Error {
        CliError::Network(msg.into()).into()
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage(_) => exit::USAGE,
            CliError::Auth(_) => exit::AUTH,
            CliError::Api { status, code, .. } => {
                if *status == 401 || is_auth_code(code) {
                    exit::AUTH
                } else {
                    exit::API
                }
            }
            CliError::Validation(_) => exit::VALIDATION,
            CliError::Network(_) => exit::NETWORK,
        }
    }

    pub fn code(&self) -> &str {
        match self {
            CliError::Usage(_) => "USAGE",
            CliError::Auth(_) => "AUTH_REQUIRED",
            CliError::Api { code, .. } => code,
            CliError::Validation(_) => "VALIDATION_FAILED",
            CliError::Network(_) => "NETWORK_ERROR",
        }
    }

    pub fn hint(&self) -> Option<&'static str> {
        match self {
            CliError::Api { code, .. } => hint_for(code),
            CliError::Auth(_) => Some("Run `times auth login` to sign in."),
            CliError::Network(_) => {
                Some("Check your connection, or set TIMES_API_URL if you use a different gateway.")
            }
            _ => None,
        }
    }
}

fn is_auth_code(code: &str) -> bool {
    matches!(
        code,
        "AUTHORIZATION_REQUIRED"
            | "INVALID_ACCESS_TOKEN"
            | "INVALID_REFRESH_TOKEN"
            | "REFRESH_TOKEN_REUSED"
            | "REFRESH_TOKEN_EXPIRED"
            | "INVALID_API_TOKEN"
            | "UNAUTHORIZED"
    )
}

pub fn hint_for(code: &str) -> Option<&'static str> {
    Some(match code {
        "AUTHORIZATION_REQUIRED" | "INVALID_ACCESS_TOKEN" | "UNAUTHORIZED" => {
            "Your session is not valid. Run `times auth login`."
        }
        "INVALID_REFRESH_TOKEN" | "REFRESH_TOKEN_REUSED" | "REFRESH_TOKEN_EXPIRED" => {
            "Your session expired. Run `times auth login` again."
        }
        "INVALID_API_TOKEN" => {
            "The token is invalid, expired or revoked. Create a new one with `times token create`."
        }
        "ORG_REQUIRED" | "DEV_ORG_REQUIRED" => {
            "Pick an organization first: `times org use <slug>`."
        }
        "ORG_ACCESS_DENIED" => "You are not a member of that organization. Run `times org list`.",
        "ORG_ROLE_FORBIDDEN" | "ORG_ADMIN_REQUIRED" => {
            "Ask an owner or admin of the organization to do this."
        }
        "INSUFFICIENT_SCOPE" | "INVALID_SCOPE" => {
            "Your token lacks the scope for this action. Create a token with the needed scopes."
        }
        "SLUG_TAKEN" | "SLUG_OWNED_ELSEWHERE" => "Pick another slug in times.aip.json.",
        "AIP_NOT_FOUND" => "Create the AIP first with `times aip create`.",
        "VERSION_EXISTS" => "Bump the version with `times manifest bump patch`.",
        "VERSION_NOT_NEWER" => {
            "The version must be higher than the last one. Run `times manifest bump patch`."
        }
        "VERSION_LOCKED" => "This version is locked. Bump the version to push changes.",
        "VERSION_NOT_READY" => {
            "Deploy this version first with `times aip deploy`."
        }
        "VERSION_NOT_FOUND" => "Deploy this version first with `times aip deploy`.",
        "AIP_IN_REVIEW" | "REVIEW_IN_PROGRESS" => {
            "A version is already in review. Wait for it, or withdraw it with `times aip withdraw`."
        }
        "SCAN_NOT_PASSED" | "BUNDLE_SCAN_FAILED" => {
            "The bundle scan did not pass. Check `times aip status` and `times aip scan`."
        }
        "LISTING_INCOMPLETE" => {
            "Finish the store listing with `times aip listing set`, then submit again."
        }
        "BUNDLE_STORAGE_UNCONFIGURED" => {
            "TIMES cannot store this upload yet. The platform host is missing bundle storage config."
        }
        "DEVELOPER_HOSTING_NO_BUNDLE" => {
            "This AIP is developer-hosted. Point hosting.entryUrl at your HTTPS origin, then deploy."
        }
        "AIP_NOT_DELETABLE" => "Published AIPs cannot be deleted.",
        "VALIDATION_FAILED" => "Check the values you passed and try again.",
        "TOKEN_NOT_FOUND" => "Run `times token list` to see token ids.",
        _ => return None,
    })
}
