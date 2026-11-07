//! muse.ai personal-agent quota, separate from the Muse Code CLI product.
//! Session credentials are supplied in Settings and kept in Windows user storage.

use crate::{
    limits::{LimitWindow, RateLimits},
    secrets,
    usage::UsageStatistics,
    worker::{Activator, LimitProvider, UsageProvider},
};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use cookie::Cookie;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    sync::LazyLock,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const ORIGIN: &str = "https://muse.ai";
const AUTH_CHECK: &str = "/api/auth/check";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36";
const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const DISCOVERY_ERROR: &str =
    "Muse's usage request could not be found. The website may have changed; refresh to retry.";
const SESSION_ERROR: &str = "Muse session expired. Sign in at muse.ai in Chrome, then replace the saved session credential.";
static CHUNKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"static/chunks/[\w~.-]+\.js").unwrap());
static SETTINGS_MODULE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.A\((\d+)\)\.then\(\(\{[^}]*Settings").unwrap());
static ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""([0-9a-f]{40,})",[^"]{0,200}"fetchSubscriptionAction""#).unwrap()
});

pub struct MuseClient {
    instance_id: String,
    action_id: Option<String>,
}
impl MuseClient {
    pub fn new(instance_id: &str) -> Self {
        Self {
            instance_id: instance_id.into(),
            action_id: None,
        }
    }

    fn fetch(&mut self, cookie: &mut Zeroizing<String>) -> Result<RateLimits> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let (status, body) = request(AUTH_CHECK, Some(cookie), None, deadline)?;
        validate_session_response(status, &body)?;
        if let Some(action) = self.action_id.clone() {
            let (status, body) = request("/", Some(cookie), Some(&action), deadline)?;
            if !stale_action(status, &body) {
                return parse_response(status, &body);
            }
            self.action_id = None;
        }
        let action = discover(cookie, deadline)?;
        let (status, body) = request("/", Some(cookie), Some(&action), deadline)?;
        if stale_action(status, &body) {
            bail!(DISCOVERY_ERROR);
        }
        let limits = parse_response(status, &body)?;
        self.action_id = Some(action);
        Ok(limits)
    }
}

pub(crate) fn secret_name(instance_id: &str) -> String {
    format!("muse.session.{instance_id}")
}
pub(crate) fn session_is_configured(instance_id: &str) -> bool {
    secrets::load(&secret_name(instance_id))
        .ok()
        .flatten()
        .map(Zeroizing::new)
        .is_some()
}

pub(crate) fn normalize_cookie(raw: &str) -> Result<Zeroizing<String>> {
    ensure!(
        !raw.contains(['\r', '\n']) && raw.len() <= 64 * 1024,
        "Paste only the Cookie header value, not a request or HAR file."
    );
    let raw = raw.trim();
    let value = if raw
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cookie:"))
    {
        raw[7..].trim()
    } else {
        raw
    };
    ensure!(
        !value.is_empty(),
        "Paste the Muse Cookie header value from Chrome."
    );
    for part in value.split(';') {
        let (name, value) = part.trim().split_once('=').ok_or_else(|| {
            anyhow::anyhow!("Use the complete Muse Cookie header value from Chrome.")
        })?;
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)),
            "Invalid Cookie header name."
        );
        ensure!(
            value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b','),
            "Invalid Cookie header value."
        );
    }
    Ok(Zeroizing::new(value.into()))
}

pub(crate) fn verify_session(raw: &str) -> Result<Zeroizing<String>> {
    let mut cookie = normalize_cookie(raw)?;
    MuseClient::new("check").fetch(&mut cookie)?;
    Ok(cookie)
}

fn validate_session_response(status: u16, body: &str) -> Result<()> {
    #[derive(Deserialize)]
    struct SessionCheck {
        ok: bool,
    }
    ensure!(
        status == 200,
        "Muse could not validate the session. Refresh to retry."
    );
    // Ignore the access token: quota requests use the cookie, and the token
    // must never enter settings, errors, or logs.
    let check: SessionCheck = serde_json::from_str(body)
        .map_err(|_| anyhow::anyhow!("Muse returned an unrecognized session response."))?;
    ensure!(check.ok, SESSION_ERROR);
    Ok(())
}

fn stale_action(status: u16, body: &str) -> bool {
    status == 404
        && body
            .to_ascii_lowercase()
            .contains("server action not found")
}

fn request(
    path: &str,
    mut cookie: Option<&mut Zeroizing<String>>,
    action: Option<&str>,
    deadline: Instant,
) -> Result<(u16, Zeroizing<String>)> {
    ensure!(
        path == "/"
            || path == AUTH_CHECK
            || (path.starts_with("/_next/static/chunks/")
                && path.ends_with(".js")
                && !path.contains("..")
                && !path.contains(['?', '#'])),
        DISCOVERY_ERROR
    );
    ensure!(
        cookie.is_none() || path == "/" || path == AUTH_CHECK,
        "Muse credentials may only be sent to muse.ai."
    );
    let timeout = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| anyhow::anyhow!("Muse usage request timed out."))?;
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    let mut req = agent
        .request(
            if action.is_some() || path == AUTH_CHECK {
                "POST"
            } else {
                "GET"
            },
            &format!("{ORIGIN}{path}"),
        )
        .set("User-Agent", USER_AGENT);
    if let Some(cookie) = cookie.as_ref() {
        req = req.set("Cookie", cookie);
    }
    let response = if let Some(action) = action {
        req.set("Accept", "text/x-component")
            .set("Next-Action", action)
            .set("Origin", ORIGIN)
            .set("Sec-Fetch-Site", "same-origin")
            .set("Sec-Fetch-Mode", "cors")
            .set("Sec-Fetch-Dest", "empty")
            .set("Content-Type", "text/plain;charset=UTF-8")
            .send_string(r#"[{"includeAgreement":false}]"#)
    } else if path == AUTH_CHECK {
        // Muse rejects this POST without same-origin Fetch Metadata, even
        // when the saved session cookie is valid.
        req.set("Accept", "application/json")
            .set("Origin", ORIGIN)
            .set("Sec-Fetch-Site", "same-origin")
            .set("Sec-Fetch-Mode", "cors")
            .set("Sec-Fetch-Dest", "empty")
            .set("Cache-Control", "no-store")
            .send_string("")
    } else {
        req.set("Accept", if path == "/" { "text/html" } else { "*/*" })
            .call()
    };
    let response = match response {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => bail!("Muse usage request could not connect or timed out."),
    };
    if let Some(cookie) = cookie.as_mut() {
        merge_response_cookies(cookie, &response, path);
    }
    let status = response.status();
    if matches!(status, 401 | 403) || matches!(status, 300..=399) {
        bail!(SESSION_ERROR);
    }
    if status == 429 {
        return Err(crate::worker::rate_limit_error(
            "Muse usage is rate limited. Try again later.",
        ));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(RESPONSE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Could not read the Muse usage response."))?;
    ensure!(
        bytes.len() <= RESPONSE_LIMIT,
        "Muse response exceeded the supported size."
    );
    let body = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("Muse returned an unrecognized response."))?;
    Ok((status, Zeroizing::new(body)))
}

fn merge_response_cookies(cookie: &mut Zeroizing<String>, response: &ureq::Response, path: &str) {
    for header in response.all("set-cookie") {
        let Ok(replacement) = Cookie::parse(header) else {
            continue;
        };
        // A pasted root-page Cookie header has no scope metadata. Only keep
        // cookies valid at both quota (/) and session-check endpoints.
        if replacement
            .domain()
            .is_some_and(|domain| !domain.eq_ignore_ascii_case("muse.ai"))
            || replacement
                .path()
                .unwrap_or(if path == "/" { "/" } else { "/api/auth" })
                != "/"
            || (replacement.name().starts_with("__Secure-") && replacement.secure() != Some(true))
            || (replacement.name().starts_with("__Host-")
                && (replacement.secure() != Some(true)
                    || replacement.domain().is_some()
                    || replacement.path() != Some("/")))
        {
            continue;
        }
        let pair = Zeroizing::new(format!("{}={}", replacement.name(), replacement.value()));
        if normalize_cookie(&pair).is_err() {
            continue;
        }
        let expired = replacement.max_age().map_or_else(
            || {
                replacement
                    .expires_datetime()
                    .is_some_and(|expires| expires.unix_timestamp() <= Utc::now().timestamp())
            },
            |age| age.whole_seconds() <= 0,
        );
        let mut updated = Zeroizing::new(
            cookie
                .split(';')
                .map(str::trim)
                .filter(|pair| {
                    pair.split_once('=')
                        .is_some_and(|(name, _)| name != replacement.name())
                })
                .collect::<Vec<_>>()
                .join("; "),
        );
        if !expired {
            if !updated.is_empty() {
                updated.push_str("; ");
            }
            updated.push_str(&pair);
        }
        if updated.len() <= 64 * 1024 {
            *cookie = updated;
        }
    }
}

fn chunk_paths(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    CHUNKS
        .find_iter(text)
        .map(|found| format!("/_next/{}", found.as_str()))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn settings_paths(text: &str) -> Vec<String> {
    let Some(module) = SETTINGS_MODULE
        .captures(text)
        .and_then(|capture| capture.get(1))
    else {
        return Vec::new();
    };
    let loader = Regex::new(&format!(
        r"[{{,\[]{},\w+=>\{{\w+\.v\(\w+=>Promise\.all\(\[([^\]]*)",
        regex::escape(module.as_str())
    ))
    .unwrap();
    loader
        .captures(text)
        .and_then(|capture| capture.get(1))
        .map(|paths| chunk_paths(paths.as_str()))
        .unwrap_or_default()
}

fn discover(cookie: &mut Zeroizing<String>, deadline: Instant) -> Result<String> {
    let (status, page) = request("/", Some(cookie), None, deadline)?;
    ensure!(status == 200, DISCOVERY_ERROR);
    let mut text = String::new();
    for path in chunk_paths(&page).into_iter().take(96) {
        let (status, chunk) = request(&path, None, None, deadline)?;
        if status != 200 {
            continue;
        }
        ensure!(text.len() + chunk.len() <= RESPONSE_LIMIT, DISCOVERY_ERROR);
        text.push_str(&chunk);
        text.push('\n');
        let paths = settings_paths(&text);
        if paths.is_empty() {
            continue;
        }
        for settings_path in paths.into_iter().take(32) {
            let (status, chunk) = request(&settings_path, None, None, deadline)?;
            if status == 200
                && let Some(action) = ACTION.captures(&chunk).and_then(|capture| capture.get(1))
            {
                return Ok(action.as_str().into());
            }
        }
        break;
    }
    bail!(DISCOVERY_ERROR)
}

fn parse_response(status: u16, body: &str) -> Result<RateLimits> {
    ensure!(
        status == 200,
        "Muse could not return usage. Refresh to retry."
    );
    let subscription = body
        .lines()
        .filter_map(|line| {
            let (id, json) = line.split_once(':')?;
            if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            let row: Value = serde_json::from_str(json).ok()?;
            (row.get("success")?.as_bool()? && row.get("subscription")?.is_object())
                .then(|| row["subscription"].clone())
        })
        .next()
        .ok_or_else(|| anyhow::anyhow!("Muse returned an unrecognized usage response."))?;
    let percent = subscription
        .pointer("/usage/percentUsed")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or_else(|| {
            anyhow::anyhow!("Muse weekly usage is unavailable; it was not reported as zero.")
        })?;
    let reset = subscription
        .pointer("/usage/resetsAt")
        .and_then(Value::as_i64)
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0));
    let plan = subscription
        .pointer("/tier/name")
        .and_then(Value::as_str)
        .or_else(|| subscription.get("usageRowLabel").and_then(Value::as_str))
        .map(str::to_owned);
    Ok(RateLimits {
        secondary: LimitWindow {
            used_percent: Some(percent.round() as u8),
            resets_at: reset,
            duration_minutes: Some(7 * 24 * 60),
        },
        plan_type: plan,
        sampled_at: Utc::now(),
        ..RateLimits::default()
    })
}

impl LimitProvider for MuseClient {
    fn read_limits(&mut self) -> Result<RateLimits> {
        let raw = Zeroizing::new(secrets::load(&secret_name(&self.instance_id))?.ok_or_else(|| anyhow::anyhow!("Connect Muse in Settings → Providers → Muse using your Chrome session credential."))?);
        let mut cookie = normalize_cookie(&raw)?;
        let original = cookie.clone();
        let result = self.fetch(&mut cookie);
        // A response may rotate credentials before a later quota request fails.
        // Persist that rotation even on failure, unless Settings replaced or
        // removed this credential while the network request was in flight.
        if cookie.as_str() != original.as_str() {
            secrets::save_if_unchanged(
                &secret_name(&self.instance_id),
                &raw,
                (!cookie.is_empty()).then_some(cookie.as_str()),
            )?;
        }
        result
    }
}
impl UsageProvider for MuseClient {
    fn load_cached_usage_statistics(&mut self, _history_days: u16) -> Result<UsageStatistics> {
        Ok(UsageStatistics::default())
    }
    fn refresh_usage_statistics(&mut self, _history_days: u16) -> Result<UsageStatistics> {
        Ok(UsageStatistics::default())
    }
}
pub struct MuseActivator;
impl Activator for MuseActivator {
    fn activate(&mut self) -> Result<()> {
        bail!("Muse has no session window to activate")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "uses a saved Muse credential and the live service; may persist renewed cookies"]
    fn live_saved_session_reads_weekly_usage() -> Result<()> {
        let instance_id = std::env::var("CODEX_MINIBAR_MUSE_TEST_INSTANCE").map_err(|_| {
            anyhow::anyhow!(
                "Set CODEX_MINIBAR_MUSE_TEST_INSTANCE to a configured Muse instance id."
            )
        })?;
        let limits = MuseClient::new(&instance_id).read_limits()?;
        ensure!(
            limits.secondary.used_percent.is_some(),
            "Muse returned no weekly usage."
        );
        ensure!(
            limits.secondary.resets_at.is_some(),
            "Muse returned no weekly reset."
        );
        Ok(())
    }

    #[test]
    fn keeps_rotated_cookies_for_following_requests_and_rejects_other_scopes() {
        let mut cookie =
            normalize_cookie("hatch_sess=test-original; test_other=preserved").unwrap();
        let response: ureq::Response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Set-Cookie: hatch_sess=test-rotated; Domain=.muse.ai; Path=/; Secure; HttpOnly\r\n",
            "Set-Cookie: __Host-test_device=test-device; Path=/; Secure; HttpOnly\r\n",
            "Set-Cookie: hatch_sess=test-wrong-domain; Domain=example.com; Path=/\r\n",
            "Set-Cookie: hatch_sess=test-wrong-path; Path=/private\r\n",
            "Set-Cookie: hatch_sess=test-default-auth-path\r\n",
            "Set-Cookie: __Host-test_device=test-insecure; Path=/\r\n",
            "Set-Cookie: invalid\r\n\r\n",
        )
        .parse()
        .unwrap();
        merge_response_cookies(&mut cookie, &response, AUTH_CHECK);
        assert_eq!(
            cookie.as_str(),
            "test_other=preserved; hatch_sess=test-rotated; __Host-test_device=test-device"
        );

        let response: ureq::Response =
            "HTTP/1.1 200 OK\r\nSet-Cookie: hatch_sess=test-next; Path=/\r\n\r\n"
                .parse()
                .unwrap();
        merge_response_cookies(&mut cookie, &response, "/");
        assert_eq!(
            cookie.as_str(),
            "test_other=preserved; __Host-test_device=test-device; hatch_sess=test-next"
        );
    }

    #[test]
    fn honors_cookie_deletion_and_max_age_precedence() {
        let mut cookie =
            normalize_cookie("hatch_sess=test-original; test_expired=old; test_keep=old").unwrap();
        let response: ureq::Response = concat!(
            "HTTP/1.1 401 Unauthorized\r\n",
            "Set-Cookie: hatch_sess=; Path=/; Max-Age=0\r\n",
            "Set-Cookie: test_expired=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT\r\n",
            "Set-Cookie: test_keep=new; Path=/; Max-Age=3600; Expires=Thu, 01 Jan 1970 00:00:00 GMT\r\n\r\n",
        ).parse().unwrap();
        merge_response_cookies(&mut cookie, &response, AUTH_CHECK);
        assert_eq!(cookie.as_str(), "test_keep=new");
    }

    #[test]
    fn validates_session_without_exposing_returned_access_tokens() {
        assert!(
            validate_session_response(
                200,
                r#"{"ok":true,"access_token":"test-token","outcome":"validated"}"#
            )
            .is_ok()
        );
        for (status, body) in [
            (200, r#"{"ok":false,"access_token":"test-token"}"#),
            (200, "test-token"),
            (503, "test-token"),
        ] {
            assert!(
                !validate_session_response(status, body)
                    .unwrap_err()
                    .to_string()
                    .contains("test-token")
            );
        }
    }

    #[test]
    fn reads_weekly_percent_plan_and_unix_second_reset() {
        let quota = parse_response(200, "0:{\"a\":\"$@1\"}\n1:{\"success\":true,\"subscription\":{\"tier\":{\"name\":\"Muse Free\"},\"usage\":{\"percentUsed\":2,\"resetsAt\":1792281600},\"topupBalance\":1000000}}\n").unwrap();
        assert_eq!(quota.secondary.used_percent, Some(2));
        assert_eq!(quota.secondary.resets_at.unwrap().timestamp(), 1792281600);
        assert_eq!(quota.plan_type.as_deref(), Some("Muse Free"));
        assert!(quota.secondary_usage_amount.is_none());
        assert!(quota.spending.is_none());
    }
    #[test]
    fn distinguishes_missing_quota_from_zero_and_exhausted() {
        for percent in [0, 100] {
            let body = format!(
                "1:{{\"success\":true,\"subscription\":{{\"usage\":{{\"percentUsed\":{percent}}}}}}}"
            );
            assert_eq!(
                parse_response(200, &body).unwrap().secondary.used_percent,
                Some(percent)
            );
        }
        assert!(parse_response(200, r#"1:{"success":true,"subscription":{"usage":{}}}"#).is_err());
        assert!(parse_response(403, "private account contents").is_err());
    }
    #[test]
    fn discovery_follows_settings_loader_and_action_reference() {
        let paths = settings_paths(
            r#"e.A(42).then(({preloadHatchSettingsData:x})=>x()),[42,e=>{e.v(x=>Promise.all(["static/chunks/settings-abc.js"]))}];"#,
        );
        assert_eq!(paths, vec!["/_next/static/chunks/settings-abc.js"]);
        assert!(ACTION.is_match(r#"createServerReference("0123456789abcdef0123456789abcdef0123456789",callServer,void 0,findSourceMapURL,"fetchSubscriptionAction")"#));
        assert!(stale_action(404, "Server action not found."));
        assert!(!stale_action(403, "Server action not found."));
    }
    #[test]
    fn validates_cookie_without_echoing_its_contents() {
        assert_eq!(
            normalize_cookie("Cookie: test_session=synthetic; test_id=fixture")
                .unwrap()
                .as_str(),
            "test_session=synthetic; test_id=fixture"
        );
        for input in [
            "",
            "Bearer synthetic",
            "test=fixture\r\nInjected: test",
            "test=private value",
        ] {
            let error = normalize_cookie(input).unwrap_err().to_string();
            assert!(
                !error.contains("synthetic")
                    && !error.contains("fixture")
                    && !error.contains("private value")
            );
        }
    }
}
