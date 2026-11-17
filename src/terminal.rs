use crate::{
    config::ConfigKind,
    engine::{Job, Status},
};
use std::{
    io::IsTerminal,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub enabled: bool,
    pub is_terminal: bool,
}

impl Theme {
    pub fn detect(choice: Option<&str>) -> Self {
        let is_term = std::io::stdout().is_terminal();
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let term_dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");

        let choice_enum = match choice.unwrap_or("auto") {
            "always" => ColorChoice::Always,
            "never" => ColorChoice::Never,
            _ => ColorChoice::Auto,
        };

        let enabled = match choice_enum {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => is_term && !no_color && !term_dumb,
        };

        Self {
            enabled,
            is_terminal: is_term,
        }
    }

    pub fn apply(&self, text: &str, code: &str) -> String {
        if self.enabled {
            format!("{code}{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, s: &str) -> String {
        self.apply(s, "\x1b[1m")
    }

    pub fn dim(&self, s: &str) -> String {
        self.apply(s, "\x1b[2m")
    }

    pub fn red(&self, s: &str) -> String {
        self.apply(s, "\x1b[31m")
    }

    pub fn green(&self, s: &str) -> String {
        self.apply(s, "\x1b[32m")
    }

    pub fn yellow(&self, s: &str) -> String {
        self.apply(s, "\x1b[33m")
    }

    pub fn magenta(&self, s: &str) -> String {
        self.apply(s, "\x1b[35m")
    }

    pub fn cyan(&self, s: &str) -> String {
        self.apply(s, "\x1b[36m")
    }

    pub fn bold_red(&self, s: &str) -> String {
        self.apply(s, "\x1b[1;31m")
    }

    pub fn bold_green(&self, s: &str) -> String {
        self.apply(s, "\x1b[1;32m")
    }

    pub fn bold_yellow(&self, s: &str) -> String {
        self.apply(s, "\x1b[1;33m")
    }

    pub fn bold_cyan(&self, s: &str) -> String {
        self.apply(s, "\x1b[1;36m")
    }

    pub fn dim_cyan(&self, s: &str) -> String {
        self.apply(s, "\x1b[2;36m")
    }

    pub fn check(&self) -> String {
        self.bold_green("✔")
    }

    pub fn cross(&self) -> String {
        self.bold_red("✖")
    }

    pub fn bullet(&self) -> String {
        self.bold_cyan("●")
    }

    pub fn arrow(&self) -> String {
        self.dim_cyan("→")
    }

    pub fn question(&self) -> String {
        self.bold_cyan("?")
    }
}

pub fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

pub fn format_job_duration(started: u64, finished: Option<u64>) -> String {
    if let Some(fin) = finished {
        if fin >= started {
            format_duration(fin - started)
        } else {
            "-".into()
        }
    } else {
        "running".into()
    }
}

pub fn format_relative_time(timestamp: u64, now: u64) -> String {
    if timestamp == 0 {
        return "-".into();
    }
    if timestamp > now {
        "just now".into()
    } else {
        let diff = now - timestamp;
        if diff < 60 {
            "just now".into()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else if diff < 86400 * 30 {
            format!("{}d ago", diff / 86400)
        } else {
            format!("{}mo ago", diff / (86400 * 30))
        }
    }
}

pub fn highlight_log_line(line: &str, theme: &Theme) -> String {
    if !theme.enabled {
        return line.to_string();
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
        theme.green(line)
    } else if trimmed.starts_with('-') && !trimmed.starts_with("---") {
        theme.red(line)
    } else if trimmed.starts_with("error:")
        || trimmed.starts_with("error[")
        || trimmed.starts_with("fatal:")
    {
        theme.bold_red(line)
    } else if trimmed.starts_with("warning:") {
        theme.bold_yellow(line)
    } else if trimmed.starts_with("building '") || trimmed.starts_with("evaluating '") {
        theme.dim_cyan(line)
    } else if line.contains(" -> ")
        && (line.contains("KiB")
            || line.contains("MiB")
            || line.contains("GiB")
            || line.contains("/nix/store"))
    {
        theme.cyan(line)
    } else {
        line.to_string()
    }
}

fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - count))
    }
}

fn label(text: &str, theme: &Theme) -> String {
    let padded = pad(text, 18);
    theme.bold(&padded)
}

pub fn print_status(status: &Status, theme: &Theme) {
    println!(
        "{} {} {}",
        theme.bold_cyan("Yonix"),
        theme.dim(&format!("v{}", status.version)),
        theme.dim("— NixOS Companion")
    );
    println!();

    let kind_badge = match status.config_kind {
        ConfigKind::Flake => theme.cyan("[flake]"),
        ConfigKind::Legacy => theme.yellow("[legacy]"),
        ConfigKind::Unknown => theme.red("[unknown]"),
    };

    println!(
        "  {} {} {}",
        label("Host:", theme),
        theme.bold(&status.host),
        kind_badge
    );
    println!(
        "  {} {}",
        label("Configuration:", theme),
        status.config_path.display()
    );

    let sys_str = status
        .current_system
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!(
        "  {} {}",
        label("Current System:", theme),
        theme.dim(&sys_str)
    );

    let changes_badge = if status.system_changes_enabled {
        theme.bold_green("✔ enabled")
    } else {
        theme.bold_yellow("✖ disabled (no privileged helper)")
    };
    println!("  {} {}", label("System Changes:", theme), changes_badge);

    let active_str = if let Some(ref act) = status.active {
        format!("{} {}", theme.bold_yellow("● active:"), theme.bold(act))
    } else {
        theme.dim("idle")
    };
    println!("  {} {}", label("Operation:", theme), active_str);

    if let Some(ref cache) = status.cache {
        let key_str = if cache.public_key.is_some() {
            theme.green("(signed)")
        } else {
            theme.dim("(unsigned)")
        };
        println!(
            "  {} {} {}",
            label("Binary Cache:", theme),
            theme.cyan(&cache.url),
            key_str
        );
    }

    println!();
    println!(
        "  {} Run '{}' for available actions.",
        theme.dim("Tip:"),
        theme.bold("yonix --help")
    );
}

pub fn print_history(jobs: &[Job], limit: Option<usize>, theme: &Theme) {
    if jobs.is_empty() {
        println!("{}", theme.dim("No operations recorded yet."));
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let display_jobs = if let Some(lim) = limit {
        &jobs[..jobs.len().min(lim)]
    } else {
        jobs
    };

    println!(
        "  {} {} {} {} {}",
        theme.bold(&pad("STATUS", 15)),
        theme.bold(&pad("ACTION", 16)),
        theme.bold(&pad("JOB ID", 14)),
        theme.bold(&pad("DURATION", 11)),
        theme.bold("STARTED")
    );
    println!("  {}", theme.dim(&"─".repeat(68)));

    for job in display_jobs {
        let (raw_badge, status_styled) = match job.status.as_str() {
            "succeeded" => ("✔ succeeded", theme.bold_green("✔ succeeded")),
            "failed" => ("✖ failed", theme.bold_red("✖ failed")),
            "running" => ("● running", theme.bold_cyan("● running")),
            "interrupted" => ("⚠ interrupted", theme.bold_yellow("⚠ interrupted")),
            other => (other, theme.dim(other)),
        };

        let short_id = if job.id.len() > 12 {
            &job.id[..12]
        } else {
            &job.id
        };

        let duration_str = format_job_duration(job.started, job.finished);
        let started_str = format_relative_time(job.started, now);

        let pad_status = 15usize.saturating_sub(raw_badge.chars().count());
        print!("  {}{}", status_styled, " ".repeat(pad_status));
        print!(" {}", theme.bold(&pad(job.action.name(), 16)));
        print!(" {}", theme.cyan(&pad(short_id, 14)));
        print!(" {}", theme.dim(&pad(&duration_str, 11)));
        println!(" {}", theme.dim(&started_str));

        if job.status == "failed" {
            if let Some(ref err) = job.error {
                let first_line = err.lines().next().unwrap_or(err);
                println!("    {} {}", theme.red("↳"), theme.dim(first_line));
            }
        }
    }

    println!();
    println!(
        "  {} View logs with '{}'",
        theme.dim("Tip:"),
        theme.bold("yonix logs <JOB_ID>")
    );
}

pub fn print_logs(id: &str, content: &str, theme: &Theme) {
    if content.trim().is_empty() {
        println!(
            "{} Log for operation {} is empty.",
            theme.dim("ℹ"),
            theme.dim(id)
        );
        return;
    }
    if theme.enabled {
        println!("{}", theme.dim(&format!("── Operation Log: {id} ──")));
        for line in content.lines() {
            println!("{}", highlight_log_line(line, theme));
        }
        println!("{}", theme.dim(&format!("── End of Log: {id} ──")));
    } else {
        print!("{content}");
    }
}

pub fn print_pair_info(link: &str, code: &str, host: &str, theme: &Theme) {
    if !theme.enabled {
        println!("{link}");
        return;
    }
    println!("{}", theme.bold_cyan("── Yonix Host Pairing ──"));
    println!();
    println!("  {:<16} {}", theme.bold("Target Host:"), theme.bold(host));
    println!(
        "  {:<16} {}",
        theme.bold("Pairing Code:"),
        theme.bold_yellow(code)
    );
    println!(
        "  {:<16} {}",
        theme.bold("Expires:"),
        theme.dim("10 minutes (single-use)")
    );
    println!();
    println!("  {}:", theme.bold("Full Pairing Link"));
    println!("  {}", theme.cyan(link));
    println!();
    println!("  To redeem on another host, run:");
    println!(
        "    {}",
        theme.bold(&format!("yonix pair-accept \"{link}\""))
    );
    println!();
}

pub fn print_json_colored(json_str: &str, theme: &Theme) {
    if !theme.enabled {
        println!("{json_str}");
        return;
    }
    for line in json_str.lines() {
        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];
        if let Some(colon_pos) = trimmed.find(':') {
            let (key_part, val_part) = trimmed.split_at(colon_pos);
            let val = &val_part[1..];
            print!("{indent}{}:", theme.cyan(key_part));
            let val_trimmed = val.trim();
            if val_trimmed.starts_with('"') {
                println!(" {}", theme.green(val_trimmed));
            } else if val_trimmed == "true" || val_trimmed == "false" || val_trimmed == "null" {
                println!(" {}", theme.yellow(val_trimmed));
            } else if val_trimmed.chars().any(|c| c.is_ascii_digit()) {
                println!(" {}", theme.magenta(val_trimmed));
            } else {
                println!("{val}");
            }
        } else {
            println!("{indent}{}", theme.dim(trimmed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_various() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(60), "1m 00s");
        assert_eq!(format_duration(125), "2m 05s");
        assert_eq!(format_duration(3665), "1h 01m");
    }

    #[test]
    fn format_relative_time_various() {
        assert_eq!(format_relative_time(0, 100), "-");
        assert_eq!(format_relative_time(100, 100), "just now");
        assert_eq!(format_relative_time(80, 100), "just now");
        assert_eq!(format_relative_time(0, 120), "-");
        assert_eq!(format_relative_time(100, 220), "2m ago");
        assert_eq!(format_relative_time(100, 7300), "2h ago");
        assert_eq!(format_relative_time(100, 172900), "2d ago");
    }

    #[test]
    fn highlight_log_lines() {
        let theme = Theme {
            enabled: true,
            is_terminal: true,
        };
        let added = highlight_log_line("+  test package", &theme);
        assert!(added.contains("\x1b[32m"));

        let removed = highlight_log_line("-  old package", &theme);
        assert!(removed.contains("\x1b[31m"));

        let err = highlight_log_line("error: failed to build", &theme);
        assert!(err.contains("\x1b[1;31m"));

        let warn = highlight_log_line("warning: unused variable", &theme);
        assert!(warn.contains("\x1b[1;33m"));

        let diff_closure = highlight_log_line("foo: 1.0 -> 1.1, +512 KiB", &theme);
        assert!(diff_closure.contains("\x1b[36m"));

        let disabled_theme = Theme {
            enabled: false,
            is_terminal: false,
        };
        let raw = highlight_log_line("+  test package", &disabled_theme);
        assert_eq!(raw, "+  test package");
    }

    #[test]
    fn theme_detect_disabled_when_not_terminal() {
        let theme = Theme {
            enabled: false,
            is_terminal: false,
        };
        assert_eq!(theme.bold("nas"), "nas");
        assert_eq!(theme.green("ok"), "ok");
    }
}
