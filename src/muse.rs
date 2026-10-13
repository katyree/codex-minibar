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
use regex::Regex;
use serde_json::Value;
use std::{
    collections::HashSet,
    io::Read,
    sync::LazyLock,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const ORIGIN: &str = "https://muse.ai";
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

    fn fetch(&mut self, cookie: &str) -> Result<RateLimits> {
        let deadline = Instant::now() + Duration::from_secs(30);
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

pub(crate) fn verify_session(raw: &str) -> Result<()> {
    let cookie = normalize_cookie(raw)?;
    MuseClient::new("check").fetch(&cookie).map(|_| ())
}

fn stale_action(status: u16, body: &str) -> bool {
    status == 404
        && body
            .to_ascii_lowercase()
            .contains("server action not found")
}

fn request(
    path: &str,
    cookie: Option<&str>,
    action: Option<&str>,
    deadline: Instant,
) -> Result<(u16, String)> {
    ensure!(
        path == "/"
            || (path.starts_with("/_next/static/chunks/")
                && path.ends_with(".js")
                && !path.contains("..")
                && !path.contains(['?', '#'])),
        DISCOVERY_ERROR
    );
    ensure!(
        cookie.is_none() || path == "/",
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
            if action.is_some() { "POST" } else { "GET" },
            &format!("{ORIGIN}{path}"),
        )
        .set("User-Agent", USER_AGENT);
    if let Some(cookie) = cookie {
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
    } else {
        req.set("Accept", if path == "/" { "text/html" } else { "*/*" })
            .call()
    };
    let response = match response {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => bail!("Muse usage request could not connect or timed out."),
    };
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
    Ok((status, body))
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

fn discover(cookie: &str, deadline: Instant) -> Result<String> {
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
        let cookie = normalize_cookie(&raw)?;
        self.fetch(&cookie)
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
