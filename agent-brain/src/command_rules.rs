use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

const READ_ONLY_AND_BUILD: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "echo",
    "printf",
    "pwd",
    "date",
    "which",
    "file",
    "stat",
    "du",
    "df",
    "diff",
    "cmp",
    "basename",
    "dirname",
    "realpath",
    "true",
    "false",
    "test",
    "[",
    "sleep",
    "uname",
    "whoami",
    "id",
    "hostname",
    "jq",
    "yq",
    "nl",
    "paste",
    "seq",
    "expr",
    "bc",
    "md5",
    "shasum",
    "sha256sum",
    "base64",
    "xxd",
    "hexdump",
    "od",
    "strings",
    "tree",
    "ps",
    "column",
    "make",
    "cargo",
    "npm",
    "pnpm",
    "yarn",
    "go",
    "gofmt",
    "rustc",
    "rustfmt",
    "pytest",
    "tsc",
    "eslint",
    "prettier",
    "agent-brain",
];

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS command_rules (
    id INTEGER PRIMARY KEY,
    command TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('allow', 'block', 'warn', 'operands')),
    trigger_spec TEXT NOT NULL DEFAULT '',
    recursive_on TEXT NOT NULL DEFAULT '',
    targets TEXT NOT NULL DEFAULT 'all',
    role TEXT NOT NULL DEFAULT '',
    value_flags TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT ''
)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: i64,
    pub command: String,
    pub mode: String,
    pub trigger: String,
    pub recursive_on: String,
    pub targets: String,
    pub role: String,
    pub value_flags: String,
    pub reason: String,
}

impl Rule {
    pub fn new(command: &str, mode: &str) -> Self {
        Self {
            id: 0,
            command: command.to_string(),
            mode: mode.to_string(),
            trigger: String::new(),
            recursive_on: String::new(),
            targets: "all".to_string(),
            role: String::new(),
            value_flags: String::new(),
            reason: String::new(),
        }
    }

    fn trigger(mut self, spec: &str) -> Self {
        self.trigger = spec.to_string();
        self
    }

    fn recursive_on(mut self, spec: &str) -> Self {
        self.recursive_on = spec.to_string();
        self
    }

    fn targets(mut self, spec: &str) -> Self {
        self.targets = spec.to_string();
        self
    }

    fn role(mut self, role: &str) -> Self {
        self.role = role.to_string();
        self
    }

    fn value_flags(mut self, flags: &str) -> Self {
        self.value_flags = flags.to_string();
        self
    }

    fn reason(mut self, reason: &str) -> Self {
        self.reason = reason.to_string();
        self
    }
}

pub fn defaults() -> Vec<Rule> {
    let mut rules: Vec<Rule> = READ_ONLY_AND_BUILD
        .iter()
        .map(|command| Rule::new(command, "allow"))
        .collect();
    for command in ["cp", "install", "rsync", "scp", "ln"] {
        rules.push(Rule::new(command, "operands").role("write").targets("last"));
    }
    for command in ["chmod", "chown", "chgrp"] {
        rules.push(
            Rule::new(command, "operands")
                .role("write")
                .targets("skip:1"),
        );
        rules.push(
            Rule::new(command, "operands")
                .role("delete")
                .targets("skip:1")
                .trigger("short:R,--recursive")
                .recursive_on("always"),
        );
    }
    rules.extend([
        Rule::new("shutdown", "block").reason("it shuts down the machine"),
        Rule::new("reboot", "block").reason("it reboots the machine"),
        Rule::new("halt", "block").reason("it halts the machine"),
        Rule::new("poweroff", "block").reason("it powers off the machine"),
        Rule::new("diskutil", "block")
            .trigger("prefix:erase,prefix:partition,reformat")
            .reason("it erases or repartitions a disk"),
        Rule::new("crontab", "warn")
            .trigger("-r")
            .reason("crontab -r removes the user's crontab"),
        Rule::new("rm", "operands")
            .role("delete")
            .recursive_on("short:r,--recursive"),
        Rule::new("unlink", "operands").role("delete"),
        Rule::new("rmdir", "operands").role("delete"),
        Rule::new("shred", "operands")
            .role("delete")
            .recursive_on("always"),
        Rule::new("truncate", "operands")
            .role("write")
            .value_flags("-s,-r"),
        Rule::new("tee", "operands").role("write"),
        Rule::new("touch", "operands").role("write"),
        Rule::new("mkdir", "operands").role("write"),
        Rule::new("mv", "operands")
            .role("delete")
            .targets("except_last"),
        Rule::new("mv", "operands").role("write").targets("last"),
        Rule::new("sed", "operands")
            .role("write")
            .trigger("short:i")
            .targets("skip:1"),
        Rule::new("perl", "operands")
            .role("write")
            .trigger("short:i")
            .targets("skip:1"),
        Rule::new("awk", "operands")
            .role("write")
            .trigger("inplace")
            .targets("skip:1")
            .value_flags("-i"),
        Rule::new("tar", "operands")
            .role("write")
            .targets("flags:-C,-f"),
        Rule::new("curl", "operands")
            .role("write")
            .targets("flags:-o,-O,--output,--output-document"),
        Rule::new("wget", "operands")
            .role("write")
            .targets("flags:-o,-O,--output,--output-document"),
        Rule::new("dd", "allow"),
        Rule::new("sqlite3", "operands").role("write"),
    ]);
    rules
}

pub fn default_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("AGENT_BRAIN_COMMAND_RULES") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".autonomic").join("command-rules.db")
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    conn.execute_batch(SCHEMA)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM command_rules", [], |row| row.get(0))?;
    if count == 0 {
        for rule in defaults() {
            insert(&conn, &rule)?;
        }
    }
    Ok(conn)
}

fn insert(conn: &Connection, rule: &Rule) -> Result<()> {
    conn.execute(
        "INSERT INTO command_rules (command, mode, trigger_spec, recursive_on, targets, role, value_flags, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            rule.command,
            rule.mode,
            rule.trigger,
            rule.recursive_on,
            rule.targets,
            rule.role,
            rule.value_flags,
            rule.reason
        ],
    )?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Vec<Rule>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT id, command, mode, trigger_spec, recursive_on, targets, role, value_flags, reason
         FROM command_rules ORDER BY id",
    )?;
    let rules = stmt
        .query_map([], |row| {
            Ok(Rule {
                id: row.get(0)?,
                command: row.get(1)?,
                mode: row.get(2)?,
                trigger: row.get(3)?,
                recursive_on: row.get(4)?,
                targets: row.get(5)?,
                role: row.get(6)?,
                value_flags: row.get(7)?,
                reason: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rules)
}

fn validate(rule: &Rule) -> Result<()> {
    if rule.command.is_empty() || rule.command.contains(['/', ' ']) {
        bail!("command must be a bare command name, for example rm");
    }
    match rule.mode.as_str() {
        "allow" | "block" | "warn" => {}
        "operands" => {
            if !matches!(rule.role.as_str(), "delete" | "write") {
                bail!("mode=operands needs role=delete or role=write");
            }
            if !(rule.targets == "all"
                || rule.targets == "last"
                || rule.targets == "except_last"
                || rule.targets.starts_with("skip:")
                || rule.targets.starts_with("flags:"))
            {
                bail!("targets must be all, last, except_last, skip:N, or flags:-x,-y");
            }
        }
        other => bail!("mode must be allow, block, warn, or operands, not {other}"),
    }
    Ok(())
}

pub fn add(path: &Path, fields: &[(String, String)]) -> Result<i64> {
    let mut rule = Rule::new("", "");
    for (key, value) in fields {
        match key.as_str() {
            "command" => rule.command = value.clone(),
            "mode" => rule.mode = value.clone(),
            "trigger" => rule.trigger = value.clone(),
            "recursive_on" => rule.recursive_on = value.clone(),
            "targets" => rule.targets = value.clone(),
            "role" => rule.role = value.clone(),
            "value_flags" => rule.value_flags = value.clone(),
            "reason" => rule.reason = value.clone(),
            other => bail!("unknown field {other}"),
        }
    }
    validate(&rule)?;
    let conn = open(path)?;
    insert(&conn, &rule)?;
    Ok(conn.last_insert_rowid())
}

pub fn remove(path: &Path, id: i64) -> Result<bool> {
    let conn = open(path)?;
    let removed = conn.execute("DELETE FROM command_rules WHERE id = ?1", params![id])?;
    Ok(removed > 0)
}

pub fn reset(path: &Path) -> Result<()> {
    let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
    conn.execute_batch("DROP TABLE IF EXISTS command_rules")?;
    drop(conn);
    open(path)?;
    Ok(())
}

pub fn is_triggered(spec: &str, args: &[&str]) -> bool {
    if spec.is_empty() {
        return true;
    }
    spec.split(',').any(|trigger| {
        args.iter().any(|arg| {
            if let Some(letter) = trigger.strip_prefix("short:") {
                let Some(letter) = letter.chars().next() else {
                    return false;
                };
                arg.starts_with('-')
                    && !arg.starts_with("--")
                    && arg.chars().skip(1).any(|c| c == letter)
            } else if let Some(prefix) = trigger.strip_prefix("prefix:") {
                arg.starts_with(prefix)
            } else if trigger == "always" {
                true
            } else {
                *arg == trigger
            }
        })
    })
}

pub fn operands<'a>(args: &[&'a str], value_flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip_value = false;
    let mut options_open = true;
    for &arg in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if options_open && arg == "--" {
            options_open = false;
            continue;
        }
        if options_open && arg.starts_with('-') && arg.len() > 1 {
            skip_value = value_flags.contains(&arg);
            continue;
        }
        out.push(arg);
    }
    out
}

pub fn select_targets<'a>(spec: &str, args: &[&'a str], value_flags: &[&str]) -> Vec<&'a str> {
    if let Some(flags) = spec.strip_prefix("flags:") {
        let flags: Vec<&str> = flags.split(',').collect();
        return args
            .windows(2)
            .filter(|pair| flags.contains(&pair[0]))
            .map(|pair| pair[1])
            .collect();
    }
    let all = operands(args, value_flags);
    if let Some(count) = spec
        .strip_prefix("skip:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        return all.get(count..).map(<[&str]>::to_vec).unwrap_or_default();
    }
    match spec {
        "last" => all.last().copied().into_iter().collect(),
        "except_last" => all
            .split_last()
            .map(|(_, rest)| rest.to_vec())
            .unwrap_or_default(),
        _ => all,
    }
}

pub fn value_flag_list(rule: &Rule) -> Vec<&str> {
    rule.value_flags
        .split(',')
        .filter(|flag| !flag.is_empty())
        .collect()
}

pub fn run_cli(args: &[String]) -> Result<()> {
    let path = default_path();
    match args.first().map(String::as_str) {
        Some("list") | None => {
            for rule in load(&path)? {
                println!(
                    "{:>4}  {:<12} {:<9} trigger={} recursive_on={} targets={} role={} value_flags={} reason={}",
                    rule.id,
                    rule.command,
                    rule.mode,
                    rule.trigger,
                    rule.recursive_on,
                    rule.targets,
                    rule.role,
                    rule.value_flags,
                    rule.reason
                );
            }
        }
        Some("add") => {
            let fields: Vec<(String, String)> = args[1..]
                .iter()
                .map(|pair| {
                    pair.split_once('=')
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .with_context(|| format!("expected key=value, got {pair}"))
                })
                .collect::<Result<_>>()?;
            let id = add(&path, &fields)?;
            println!("added rule {id} to {}", path.display());
        }
        Some("remove") => {
            let id: i64 = args
                .get(1)
                .context("usage: agent-brain rules remove <id>")?
                .parse()
                .context("id must be a number")?;
            if remove(&path, id)? {
                println!("removed rule {id}");
            } else {
                bail!("no rule with id {id}");
            }
        }
        Some("reset") => {
            reset(&path)?;
            println!("reset rules to defaults in {}", path.display());
        }
        Some(other) => bail!("unknown rules command {other}; use list, add, remove, or reset"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_seed_an_empty_database_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rules.db");
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), defaults().len());
        assert_eq!(load(&path).unwrap().len(), defaults().len());
    }

    #[test]
    fn added_rule_round_trips_and_removes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rules.db");
        let id = add(
            &path,
            &[
                ("command".into(), "docker".into()),
                ("mode".into(), "block".into()),
                ("reason".into(), "no containers".into()),
            ],
        )
        .unwrap();
        let rule = load(&path)
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap();
        assert_eq!(rule.command, "docker");
        assert_eq!(rule.reason, "no containers");
        assert!(remove(&path, id).unwrap());
        assert!(!remove(&path, id).unwrap());
    }

    #[test]
    fn rejects_invalid_rules() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rules.db");
        assert!(add(
            &path,
            &[
                ("command".into(), "rm".into()),
                ("mode".into(), "nuke".into())
            ]
        )
        .is_err());
        assert!(add(
            &path,
            &[
                ("command".into(), "rm".into()),
                ("mode".into(), "operands".into())
            ]
        )
        .is_err());
        assert!(add(
            &path,
            &[
                ("command".into(), "/bin/rm".into()),
                ("mode".into(), "allow".into())
            ]
        )
        .is_err());
        assert!(add(&path, &[("bogus".into(), "x".into())]).is_err());
    }

    #[test]
    fn triggers_match_short_clusters_prefixes_and_exact_tokens() {
        assert!(is_triggered("short:r", &["-rf", "x"]));
        assert!(!is_triggered("short:r", &["-f", "x"]));
        assert!(is_triggered("--recursive", &["--recursive"]));
        assert!(is_triggered("prefix:erase", &["eraseDisk"]));
        assert!(!is_triggered("prefix:erase", &["list"]));
        assert!(is_triggered("", &[]));
    }

    #[test]
    fn target_selection_follows_the_spec() {
        let args = ["-m", "755", "file.txt", "dir"];
        assert_eq!(select_targets("skip:1", &args, &["-m"]), vec!["dir"]);
        assert_eq!(select_targets("last", &args, &["-m"]), vec!["dir"]);
        assert_eq!(
            select_targets("except_last", &args, &["-m"]),
            vec!["file.txt"]
        );
        let tar = ["-x", "-C", "/", "-f", "a.tar"];
        assert_eq!(select_targets("flags:-C,-f", &tar, &[]), vec!["/", "a.tar"]);
    }
}
