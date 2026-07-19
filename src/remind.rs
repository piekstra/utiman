//! Daily bill reminders via a launchd agent (macOS).
//!
//! `utiman remind install` writes a LaunchAgent that runs `utiman check
//! --notify` once a day, so due-soon bills raise a notification without the
//! dashboard (or any terminal) being open. Everything routes through the same
//! `check` the dashboard and cron use; this module only manages the schedule.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// launchd label / plist basename — also the reverse-DNS id launchctl uses.
pub const LABEL: &str = "com.piekstra.utiman.check";

/// A configured reminder: the daily time and the "due soon" window it checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Reminder {
    pub hour: u32,
    pub minute: u32,
    pub within: i64,
}

/// `~/Library/LaunchAgents/<LABEL>.plist`.
pub fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// Parse `HH:MM` (24-hour) into (hour, minute).
pub fn parse_time(s: &str) -> Result<(u32, u32)> {
    let (h, m) = s
        .split_once(':')
        .with_context(|| format!("time must look like HH:MM, got {s:?}"))?;
    let hour: u32 = h.trim().parse().context("bad hour")?;
    let minute: u32 = m.trim().parse().context("bad minute")?;
    if hour > 23 || minute > 59 {
        bail!("time out of range (00:00–23:59): {s:?}");
    }
    Ok((hour, minute))
}

/// Render the LaunchAgent plist that runs `<exe> check --notify --within N`
/// daily at the given time. Pure — the exe path and values are the only inputs.
pub fn plist_xml(exe: &str, r: Reminder) -> String {
    // Log to the user's cache dir so a failed run leaves a breadcrumb.
    let log = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("utiman-remind.log");
    let log = log.display();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>check</string>
    <string>--notify</string>
    <string>--within</string>
    <string>{within}</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Hour</key><integer>{hour}</integer>
    <key>Minute</key><integer>{minute}</integer>
  </dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        within = r.within,
        hour = r.hour,
        minute = r.minute,
    )
}

/// Read the current schedule back out of an installed plist (best-effort, so
/// the dashboard/status can show what's set without shelling out to launchctl).
pub fn parse_status(plist: &str) -> Option<Reminder> {
    let hour = tag_after(plist, "Hour")?;
    let minute = tag_after(plist, "Minute")?;
    // `within` is the argument right after the "--within" element.
    let within = plist
        .split("--within")
        .nth(1)
        .and_then(|rest| tag_between(rest, "<string>", "</string>"))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(5);
    Some(Reminder {
        hour,
        minute,
        within,
    })
}

/// Value of the `<integer>` immediately following `<key>NAME</key>`.
fn tag_after(plist: &str, key: &str) -> Option<u32> {
    let rest = plist.split(&format!("<key>{key}</key>")).nth(1)?;
    tag_between(rest, "<integer>", "</integer>")?
        .trim()
        .parse()
        .ok()
}

fn tag_between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

/// Path to the running utiman binary, for the plist's ProgramArguments.
fn exe_path() -> Result<String> {
    let p = std::env::current_exe().context("cannot resolve utiman's own path")?;
    Ok(p.to_string_lossy().into_owned())
}

/// Install (or replace) the daily reminder and load it into launchd.
pub fn install(r: Reminder) -> Result<()> {
    if std::env::consts::OS != "macos" {
        bail!(
            "reminders use launchd (macOS only); on Linux, run `utiman check --notify` from cron"
        );
    }
    let path = plist_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(&path, plist_xml(&exe_path()?, r))
        .with_context(|| format!("cannot write {}", path.display()))?;
    // Reload: unload any prior copy (ignore failure), then load the new one.
    let _ = launchctl(&["unload", &path.to_string_lossy()]);
    launchctl(&["load", &path.to_string_lossy()])?;
    Ok(())
}

/// Remove the reminder and unload it from launchd. Idempotent.
pub fn uninstall() -> Result<()> {
    let path = plist_path();
    if path.exists() {
        let _ = launchctl(&["unload", &path.to_string_lossy()]);
        std::fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
    }
    Ok(())
}

/// Current reminder, if one is installed.
pub fn status() -> Option<Reminder> {
    parse_status(&std::fs::read_to_string(plist_path()).ok()?)
}

fn launchctl(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .context("launchctl not available")?;
    if !out.status.success() {
        bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time_ok_and_bounds() {
        assert_eq!(parse_time("09:00").unwrap(), (9, 0));
        assert_eq!(parse_time("23:59").unwrap(), (23, 59));
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("9").is_err());
        assert!(parse_time("09:60").is_err());
    }

    #[test]
    fn plist_roundtrips_through_parse_status() {
        let r = Reminder {
            hour: 8,
            minute: 30,
            within: 7,
        };
        let xml = plist_xml("/usr/local/bin/utiman", r);
        assert!(xml.contains("<string>/usr/local/bin/utiman</string>"));
        assert!(xml.contains(&format!("<string>{LABEL}</string>")));
        assert_eq!(parse_status(&xml), Some(r));
    }

    #[test]
    fn parse_status_defaults_within_when_absent() {
        // A minimal plist with the time but no --within still yields a value.
        let xml = "<key>Hour</key><integer>6</integer><key>Minute</key><integer>15</integer>";
        assert_eq!(
            parse_status(xml),
            Some(Reminder {
                hour: 6,
                minute: 15,
                within: 5
            })
        );
    }
}
