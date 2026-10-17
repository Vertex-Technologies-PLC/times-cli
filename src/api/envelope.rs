use serde_json::Value;

use crate::error::CliError;

pub fn parse(status: u16, body: &str) -> Result<Value, CliError> {
    let ok = (200..300).contains(&status);
    let json: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) if ok && body.trim().is_empty() => return Ok(Value::Null),
        Err(_) => {
            return Err(CliError::Api {
                status,
                code: format!("HTTP_{status}"),
                message: fallback_message(status, body),
                details: None,
            })
        }
    };

    if let Some(success) = json.get("success").and_then(Value::as_bool) {
        if success && ok {
            return Ok(json.get("data").cloned().unwrap_or(Value::Null));
        }
        return Err(CliError::Api {
            status,
            code: str_field(&json, "code").unwrap_or_else(|| format!("HTTP_{status}")),
            message: str_field(&json, "message").unwrap_or_else(|| fallback_message(status, "")),
            details: json.get("details").filter(|d| !d.is_null()).cloned(),
        });
    }

    if json.get("internalCode").is_some() || json.get("statusCode").is_some() {
        let erp_ok = json
            .get("status")
            .and_then(Value::as_str)
            .map(|s| s.eq_ignore_ascii_case("success"))
            .unwrap_or(ok);
        if erp_ok && ok {
            return Ok(json.get("data").cloned().unwrap_or(Value::Null));
        }
        let code = str_field(&json, "internalCode")
            .map(|c| strip_status_suffix(&c))
            .unwrap_or_else(|| format!("HTTP_{status}"));
        return Err(CliError::Api {
            status,
            code,
            message: str_field(&json, "message").unwrap_or_else(|| fallback_message(status, "")),
            details: json.get("errors").filter(|d| !d.is_null()).cloned(),
        });
    }

    if ok {
        return Ok(json);
    }

    Err(CliError::Api {
        status,
        code: str_field(&json, "code")
            .or_else(|| str_field(&json, "error"))
            .unwrap_or_else(|| format!("HTTP_{status}")),
        message: str_field(&json, "message")
            .or_else(|| str_field(&json, "title"))
            .unwrap_or_else(|| fallback_message(status, "")),
        details: None,
    })
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn strip_status_suffix(code: &str) -> String {
    match code.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 3 && tail.chars().all(|c| c.is_ascii_digit()) => {
            head.to_string()
        }
        _ => code.to_string(),
    }
}

fn fallback_message(status: u16, body: &str) -> String {
    let trimmed = body.trim();
    if !trimmed.is_empty() && trimmed.len() < 200 {
        return format!("Request failed with HTTP {status}: {trimmed}");
    }
    match status {
        401 => "You are not signed in.".into(),
        403 => "You do not have access to this.".into(),
        404 => "Not found.".into(),
        429 => "Too many requests. Wait a moment and try again.".into(),
        500..=599 => format!("The TIMES API had a problem (HTTP {status}). Try again later."),
        _ => format!("Request failed with HTTP {status}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auth_success_returns_data() {
        let body = json!({"success": true, "code": "OK", "message": "ok", "data": {"a": 1}});
        assert_eq!(parse(200, &body.to_string()).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn auth_error_keeps_code_and_details() {
        let body = json!({"success": false, "code": "slow_down", "message": "slow", "details": {"interval": 10}});
        match parse(400, &body.to_string()).unwrap_err() {
            CliError::Api {
                status,
                code,
                details,
                ..
            } => {
                assert_eq!(status, 400);
                assert_eq!(code, "slow_down");
                assert_eq!(details.unwrap()["interval"], 10);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn erp_success_returns_data() {
        let body = json!({"internalCode": "SUCCESS-200", "statusCode": 200, "status": "SUCCESS", "message": "ok", "data": {"items": []}});
        assert_eq!(parse(200, &body.to_string()).unwrap(), json!({"items": []}));
    }

    #[test]
    fn erp_error_strips_status_suffix() {
        let body = json!({"internalCode": "SLUG_TAKEN-409", "statusCode": 409, "status": "ERROR", "message": "Slug is taken"});
        let err = parse(409, &body.to_string()).unwrap_err();
        assert_eq!(err.code(), "SLUG_TAKEN");
        assert_eq!(err.to_string(), "Slug is taken");
    }

    #[test]
    fn plain_json_and_empty_bodies() {
        assert_eq!(parse(200, "[1,2]").unwrap(), json!([1, 2]));
        assert_eq!(parse(204, "").unwrap(), Value::Null);
        assert_eq!(parse(502, "<html>").unwrap_err().code(), "HTTP_502");
    }

    #[test]
    fn suffix_only_stripped_when_numeric() {
        assert_eq!(
            strip_status_suffix("DEV_ORG_REQUIRED-403"),
            "DEV_ORG_REQUIRED"
        );
        assert_eq!(strip_status_suffix("A-B"), "A-B");
        assert_eq!(strip_status_suffix("PLAIN"), "PLAIN");
    }
}
