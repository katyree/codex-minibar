//! Junie uses the JetBrains AI account credit pool, shared with AI Assistant.
//! Central owns authentication; the IDE file is a timestamped, read-only fallback.

use crate::{
    providers::{
        drivers::{
            Driver, FolderField, InstallSource, InstallStatus, SettingsCopy, Sources, display_path,
        },
        instances::ProviderInstance,
        support::{CliSearch, find_cli},
        worker::{LimitProvider, UsageProvider, WorkerHandle, WorkerOptions},
    },
    quota::limits::{LimitWindow, RateLimits, UsageAmount},
    usage::UsageStatistics,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const QUOTA_FILE: &str = "AIAssistantQuotaManager2.xml";

#[derive(Clone, Default)]
pub struct JunieClient {
    binary_path: Option<PathBuf>,
    cache_path: Option<PathBuf>,
}

impl JunieClient {
    fn new(instance: &ProviderInstance) -> Self {
        Self {
            binary_path: instance.binary_path.clone(),
            // Older settings used the same field for Central and the IDE cache.
            cache_path: instance
                .app_path
                .as_ref()
                .or(instance.binary_path.as_ref())
                .cloned(),
        }
    }
}

fn central_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        let candidates = if path.is_file() {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
            {
                return None;
            }
            vec![path.to_path_buf()]
        } else {
            vec![path.join("central.exe"), path.join("bin/central.exe")]
        };
        return candidates
            .into_iter()
            .find(|path| path.is_file())
            .and_then(|path| fs::canonicalize(path).ok());
    }
    let known =
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".local/bin/central.exe"));
    find_cli(
        None,
        &CliSearch {
            names: &["central.exe"],
            known: known.into_iter().collect(),
            user_bins: false,
        },
    )
    .into_iter()
    .next()
    .and_then(|path| fs::canonicalize(path).ok())
}

fn cache_path(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return [
            path.to_path_buf(),
            path.join(QUOTA_FILE),
            path.join("options").join(QUOTA_FILE),
        ]
        .into_iter()
        .find(|path| path.is_file() && path.file_name().is_some_and(|name| name == QUOTA_FILE));
    }
    let root = directories::BaseDirs::new()?.config_dir().join("JetBrains");
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("options").join(QUOTA_FILE))
        .filter(|path| path.is_file())
        .max_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok())
}

pub(crate) static DRIVER: Driver = Driver {
    start: start_worker,
    detect,
    sources: Sources::APP_AND_CLI,
    copy: settings_copy,
    forget_secrets: None,
    consume_reset: None,
};

fn start_worker(instance: &ProviderInstance, options: WorkerOptions) -> Result<WorkerHandle> {
    let client = JunieClient::new(instance);
    Ok(crate::providers::worker::start_worker(
        client.clone(),
        client,
        None,
        options,
    ))
}

fn detect(instance: &ProviderInstance) -> InstallStatus {
    let client = JunieClient::new(instance);
    let cli = central_path(client.binary_path.as_deref());
    let app = cache_path(client.cache_path.as_deref());
    InstallStatus {
        used: if cli.is_some() {
            Some(InstallSource::Cli)
        } else {
            app.as_ref().map(|_| InstallSource::App)
        },
        cli: cli.as_deref().map(display_path),
        app: app.as_deref().map(display_path),
        ..InstallStatus::none(instance)
    }
}

fn settings_copy() -> SettingsCopy {
    SettingsCopy {
        app: crate::i18n::tr("junie-ide-cache"),
        cli: "JetBrains Central",
        folders: vec![FolderField::binary(
            crate::i18n::tr("junie-source-path"),
            crate::i18n::tr("junie-source-path-help"),
            r"C:\Users\you\.local\bin",
        )],
        ..SettingsCopy::new(crate::i18n::tr("junie-provider-help"))
    }
}

impl LimitProvider for JunieClient {
    fn read_limits(&mut self) -> Result<RateLimits> {
        if let Some(executable) = central_path(self.binary_path.as_deref()) {
            // A failed live request must never silently switch to another IDE/account.
            return parse_central(&read_central(&executable)?);
        }
        let path = cache_path(self.cache_path.as_deref()).context("Junie quota was not found. Install and sign in to JetBrains Central, or open the IDE's Junie License & quota view and save its configuration.")?;
        let modified = fs::metadata(&path)?.modified()?;
        let contents =
            fs::read_to_string(path).context("Could not read the saved JetBrains AI quota")?;
        parse_cache(&contents, DateTime::<Utc>::from(modified))
    }
}

fn read_central(executable: &Path) -> Result<Vec<u8>> {
    let mut command = Command::new(executable);
    command
        .args(["limit", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .context("Could not start JetBrains Central")?;
    let stdout = child
        .stdout
        .take()
        .context("Could not read JetBrains Central output")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("JetBrains Central quota request timed out or could not be checked");
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Could not read JetBrains Central output"))??;
    if !status.success() {
        bail!(
            "JetBrains Central could not read quota. Run central limit in a terminal to check your sign-in and connection."
        );
    }
    if bytes.len() > 1024 * 1024 {
        bail!("JetBrains Central quota response is too large");
    }
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum QuotaNumber {
    Number(f64),
    Text(String),
}
impl QuotaNumber {
    fn value(self) -> Result<f64> {
        match self {
            Self::Number(value) => Ok(value),
            Self::Text(value) => value.parse().context("Invalid JetBrains credit amount"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CentralQuota {
    used_dollars: QuotaNumber,
    max_dollars: QuotaNumber,
    refill_next: Option<i64>,
}

fn limits(
    used: f64,
    limit: f64,
    reset: Option<DateTime<Utc>>,
    sampled_at: DateTime<Utc>,
    cached: bool,
) -> Result<RateLimits> {
    if !used.is_finite() || used < 0.0 || !limit.is_finite() || limit <= 0.0 {
        bail!("JetBrains AI quota is unavailable or invalid");
    }
    Ok(RateLimits {
        secondary: LimitWindow {
            used_percent: Some((used / limit * 100.0).round().clamp(0.0, 100.0) as u8),
            resets_at: reset,
            duration_minutes: None,
        },
        secondary_usage_amount: Some(UsageAmount { used, limit }),
        secondary_limit_name: Some("AI credits".into()),
        plan_type: Some(
            if cached {
                "JetBrains AI · IDE cache"
            } else {
                "JetBrains AI · Central"
            }
            .into(),
        ),
        sampled_at,
        ..RateLimits::default()
    })
}

fn parse_central(bytes: &[u8]) -> Result<RateLimits> {
    let quota: CentralQuota = serde_json::from_slice(bytes).map_err(|_| {
        anyhow::anyhow!("JetBrains Central returned an unrecognized quota response")
    })?;
    limits(
        quota.used_dollars.value()?,
        quota.max_dollars.value()?,
        quota
            .refill_next
            .and_then(DateTime::<Utc>::from_timestamp_millis),
        Utc::now(),
        false,
    )
}

fn xml_attribute(tag: &str, name: &str) -> Option<String> {
    // JetBrains saves each option as name/value attributes, in either order.
    let mut rest = tag;
    while let Some(start) = rest.find(name) {
        let before = &rest[..start];
        let after = &rest[start + name.len()..];
        rest = after;
        if !before.ends_with(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let value = &value[1..];
        let end = value.find(quote)?;
        return decode_xml(&value[..end]);
    }
    None
}

fn decode_xml(value: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find(';')?;
        let entity = &rest[..end];
        decoded.push(match entity {
            "quot" => '"',
            "apos" => '\'',
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    entity.strip_prefix('#')?.parse().ok()?
                };
                char::from_u32(code)?
            }
        });
        rest = &rest[end + 1..];
    }
    decoded.push_str(rest);
    Some(decoded)
}

#[derive(Deserialize)]
struct CachedQuota {
    #[serde(rename = "type")]
    kind: String,
    current: QuotaNumber,
    maximum: QuotaNumber,
}
#[derive(Deserialize)]
struct NextRefill {
    #[serde(rename = "type")]
    kind: String,
    next: Option<DateTime<Utc>>,
}

fn parse_cache(xml: &str, sampled_at: DateTime<Utc>) -> Result<RateLimits> {
    let mut quota = None;
    let mut reset = None;
    for option in xml.split("<option").skip(1) {
        let Some(end) = option.find('>') else {
            continue;
        };
        let tag = &option[..end];
        match xml_attribute(tag, "name").as_deref() {
            Some("quotaInfo") => quota = xml_attribute(tag, "value"),
            Some("nextRefill") => reset = xml_attribute(tag, "value"),
            _ => {}
        }
    }
    let quota: CachedQuota=serde_json::from_str(&quota.context("JetBrains AI quota has not been cached. Refresh Junie's License & quota view and save the IDE configuration.")?).map_err(|_| anyhow::anyhow!("The saved JetBrains AI quota is invalid"))?;
    if quota.kind != "Available" {
        bail!("JetBrains AI quota is unavailable");
    }
    let reset = reset
        .and_then(|json| serde_json::from_str::<NextRefill>(&json).ok())
        .filter(|refill| refill.kind == "Known")
        .and_then(|refill| refill.next);
    limits(
        quota.current.value()?,
        quota.maximum.value()?,
        reset,
        sampled_at,
        true,
    )
}

impl UsageProvider for JunieClient {
    fn load_cached_usage_statistics(&mut self, _history_days: u16) -> Result<UsageStatistics> {
        Ok(UsageStatistics::default())
    }
    fn refresh_usage_statistics(&mut self, _history_days: u16) -> Result<UsageStatistics> {
        Ok(UsageStatistics::default())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn central_reports_shared_credits_and_refill_without_account_details() {
        let quota=parse_central(br#"{"usedDollars":"4.28","maxDollars":"40","refillNext":1793491199999,"email":"test@example.invalid"}"#).unwrap();
        assert_eq!(quota.secondary.used_percent, Some(11));
        assert_eq!(
            quota.secondary_usage_amount,
            Some(UsageAmount {
                used: 4.28,
                limit: 40.0
            })
        );
        assert_eq!(
            quota.secondary.resets_at.unwrap().timestamp_millis(),
            1793491199999
        );
        assert!(quota.account_name.is_none());
        assert!(quota.primary.used_percent.is_none());
    }
    #[test]
    fn unavailable_or_invalid_quota_is_not_zero_usage() {
        for json in [
            r#"{}"#,
            r#"{"usedDollars":0,"maxDollars":0}"#,
            r#"{"usedDollars":-1,"maxDollars":40}"#,
        ] {
            assert!(parse_central(json.as_bytes()).is_err());
        }
    }
    #[test]
    fn cache_preserves_timestamp_and_uses_refill_not_validity() {
        let sample = DateTime::from_timestamp(1700000000, 0).unwrap();
        let quota=parse_cache(r#"<option value='{"type":"Available","current":10,"maximum":40,"until":"2030-01-01T00:00:00Z"}' name="quotaInfo"/><option name="nextRefill" value="{&quot;type&quot;:&quot;Known&quot;,&quot;next&quot;:&quot;2026-11-01T00:00:00Z&quot;}"/>"#,sample).unwrap();
        assert_eq!(quota.secondary.used_percent, Some(25));
        assert_eq!(quota.sampled_at, sample);
        assert_eq!(
            quota.secondary.resets_at.unwrap().to_rfc3339(),
            "2026-11-01T00:00:00+00:00"
        );
    }
    #[test]
    fn explicit_cache_does_not_select_an_unrelated_cli() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(QUOTA_FILE);
        fs::write(&path, "test").unwrap();
        assert!(central_path(Some(&path)).is_none());
        assert_eq!(cache_path(Some(&path)), Some(path));
    }
}
