//! Versionamento das notas da Athena com Git, no lugar do plugin obsidian-git.
//!
//! Os plugins do Obsidian só rodam dentro do Obsidian; a Ayla não os executa. Para o Git, ela
//! roda o `git` na pasta do cofre seguindo as configurações do próprio obsidian-git
//! (`.obsidian/plugins/obsidian-git/data.json`): mensagem de commit, formato da data, intervalo
//! automático, puxar ao abrir, puxar antes de enviar e merge/rebase. Nunca força envio e não roda
//! enquanto o Obsidian estiver aberto (o plugin já cuida disso lá). As credenciais são as do
//! próprio Git (remoto, SSH ou gerenciador de credenciais); a Ayla não guarda nenhuma.
use serde::Serialize;
use std::{
    path::Path,
    process::{Command, Stdio},
};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Settings {
    /// Minutos entre sincronizações automáticas (0: só manual).
    pub interval: u64,
    pub message: String,
    pub date_format: String,
    pub pull_on_open: bool,
    pub push: bool,
    pub pull_before_push: bool,
    pub rebase: bool,
    /// O cofre tem o obsidian-git configurado.
    pub plugin: bool,
}

impl Default for Settings {
    /// Padrões do obsidian-git.
    fn default() -> Self {
        Self {
            interval: 0,
            message: "vault backup: {{date}}".into(),
            date_format: "YYYY-MM-DD HH:mm:ss".into(),
            pull_on_open: false,
            push: true,
            pull_before_push: true,
            rebase: false,
            plugin: false,
        }
    }
}

/// Configurações do obsidian-git no cofre, ou os padrões dele.
pub fn settings(root: &Path) -> Settings {
    let mut out = Settings::default();
    let plugins = root.join(".obsidian/community-plugins.json");
    out.plugin = std::fs::read_to_string(plugins).is_ok_and(|t| t.contains("\"obsidian-git\""));
    let Ok(text) = std::fs::read_to_string(root.join(".obsidian/plugins/obsidian-git/data.json")) else { return out };
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) else { return out };
    if let Some(minutes) = data["autoSaveInterval"].as_u64() {
        out.interval = minutes.min(24 * 60);
    }
    if let Some(message) = data["autoCommitMessage"].as_str().or_else(|| data["commitMessage"].as_str()).filter(|m| !m.trim().is_empty()) {
        out.message = message.chars().take(500).collect();
    }
    if let Some(format) = data["commitDateFormat"].as_str().filter(|f| !f.trim().is_empty()) {
        out.date_format = format.chars().take(100).collect();
    }
    out.pull_on_open = data["autoPullOnBoot"].as_bool().unwrap_or(false);
    out.push = !data["disablePush"].as_bool().unwrap_or(false);
    out.pull_before_push = data["pullBeforePush"].as_bool().unwrap_or(true);
    out.rebase = data["syncMethod"].as_str() == Some("rebase");
    out
}

/// Formato de data do obsidian-git (Moment.js) para o chrono: YYYY, MM, DD, HH, mm, ss.
pub fn chrono_format(moment: &str) -> String {
    let mut out = String::new();
    let mut rest = moment;
    let tokens = [("YYYY", "%Y"), ("YY", "%y"), ("MM", "%m"), ("DD", "%d"), ("HH", "%H"), ("hh", "%I"), ("mm", "%M"), ("ss", "%S"), ("A", "%p")];
    'outer: while !rest.is_empty() {
        for (token, format) in tokens {
            if let Some(after) = rest.strip_prefix(token) {
                out.push_str(format);
                rest = after;
                continue 'outer;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        if c == '%' {
            out.push_str("%%");
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Mensagem do commit com `{{date}}`, `{{hostname}}` e `{{numFiles}}`.
pub fn commit_message(settings: &Settings, now: chrono::NaiveDateTime, files: usize) -> String {
    let host = std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_default();
    settings
        .message
        .replace("{{date}}", &now.format(&chrono_format(&settings.date_format)).to_string())
        .replace("{{hostname}}", &host)
        .replace("{{numFiles}}", &files.to_string())
}

fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .stdin(Stdio::null());
    command
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = git(root).args(args).output().map_err(|_| "O Git não está disponível neste computador".to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let detail = String::from_utf8_lossy(&out.stderr);
        Err(clean_error(&detail))
    }
}

/// Erro do Git em uma linha, sem credenciais que possam estar no endereço do remoto.
pub fn clean_error(detail: &str) -> String {
    let line = detail.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("o Git recusou a operação");
    let mut out = String::new();
    for word in line.split(' ') {
        if let Some((scheme, rest)) = word.split_once("://") {
            if let Some((_, host)) = rest.split_once('@') {
                out.push_str(&format!("{scheme}://{host} "));
                continue;
            }
        }
        out.push_str(word);
        out.push(' ');
    }
    out.trim().chars().take(300).collect()
}

/// Cofre com repositório Git próprio na raiz.
pub fn is_repo(root: &Path) -> bool {
    root.join(".git").exists()
}

/// Obsidian aberto (instalação comum ou Flatpak): a Ayla não sincroniza ao mesmo tempo.
pub fn obsidian_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else { return false };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("comm")).is_ok_and(|name| name.trim().eq_ignore_ascii_case("obsidian"))
    })
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub repo: bool,
    pub branch: String,
    /// Arquivos alterados ainda sem commit.
    pub changes: usize,
    pub remote: bool,
    pub settings: Option<Settings>,
}

pub fn status(root: &Path) -> Status {
    if !is_repo(root) {
        return Status::default();
    }
    let changes = run(root, &["status", "--porcelain"]).map(|o| o.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0);
    Status {
        repo: true,
        branch: run(root, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| b.trim().to_string()).unwrap_or_default(),
        changes,
        remote: run(root, &["remote"]).is_ok_and(|r| !r.trim().is_empty()),
        settings: Some(settings(root)),
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Outcome {
    pub committed: usize,
    pub pulled: bool,
    pub pushed: bool,
    /// Texto para a pessoa: o que aconteceu ou o que impediu.
    pub message: String,
}

/// Commit das mudanças, puxar (se configurado) e enviar. Em conflito, desfaz o merge/rebase em
/// andamento e avisa, sem perder nada do que está na pasta.
pub fn sync(root: &Path, settings: &Settings, now: chrono::NaiveDateTime) -> Result<Outcome, String> {
    if !is_repo(root) {
        return Err("Esta pasta não é um repositório Git.".into());
    }
    let mut outcome = Outcome::default();
    let changed: Vec<String> = run(root, &["status", "--porcelain"])?.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
    if !changed.is_empty() {
        run(root, &["add", "-A"])?;
        run(root, &["commit", "-q", "-m", &commit_message(settings, now, changed.len())])?;
        outcome.committed = changed.len();
    }
    let remote = run(root, &["remote"]).ok().and_then(|r| r.lines().next().map(|l| l.trim().to_string())).filter(|r| !r.is_empty());
    let has_remote = remote.is_some();
    // Ramo sem vínculo com o remoto (primeiro envio): não há o que puxar; envia criando o vínculo.
    let upstream = run(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok();
    if has_remote && upstream && settings.pull_before_push {
        let pulled = if settings.rebase { run(root, &["pull", "-q", "--rebase"]) } else { run(root, &["pull", "-q", "--no-rebase", "--no-edit"]) };
        if let Err(error) = pulled {
            if root.join(".git/MERGE_HEAD").exists() {
                let _ = run(root, &["merge", "--abort"]);
            }
            if root.join(".git/rebase-merge").exists() || root.join(".git/rebase-apply").exists() {
                let _ = run(root, &["rebase", "--abort"]);
            }
            outcome.message = format!("Não deu para puxar do GitHub: {error}. As notas ficaram como estavam; resolva o conflito no Obsidian ou no terminal.");
            return Ok(outcome);
        }
        outcome.pulled = true;
    }
    if has_remote && settings.push {
        let pushed = if upstream { run(root, &["push", "-q"]) } else { run(root, &["push", "-q", "-u", remote.as_deref().unwrap_or("origin"), "HEAD"]) };
        match pushed {
            Ok(_) => outcome.pushed = true,
            Err(error) => {
                outcome.message = format!("Commit feito, mas o envio falhou: {error}");
                return Ok(outcome);
            }
        }
    }
    outcome.message = match (outcome.committed, outcome.pushed, has_remote) {
        (0, true, _) => "Tudo em dia com o GitHub.".into(),
        (n, true, _) => format!("{n} {} versionado{} e enviado{}.", if n == 1 { "arquivo" } else { "arquivos" }, if n == 1 { "" } else { "s" }, if n == 1 { "" } else { "s" }),
        (0, false, false) => "Nada a versionar (sem remoto configurado).".into(),
        (n, false, _) => format!("{n} {} versionado{} localmente.", if n == 1 { "arquivo" } else { "arquivos" }, if n == 1 { "" } else { "s" }),
    };
    Ok(outcome)
}

/// Só puxar (opção "puxar ao abrir" do obsidian-git). Em conflito, desfaz e avisa.
pub fn pull(root: &Path, settings: &Settings) -> Result<(), String> {
    if !is_repo(root) || run(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_err() {
        return Ok(());
    }
    let result = if settings.rebase { run(root, &["pull", "-q", "--rebase", "--autostash"]) } else { run(root, &["pull", "-q", "--no-rebase", "--no-edit"]) };
    if result.is_err() {
        if root.join(".git/MERGE_HEAD").exists() {
            let _ = run(root, &["merge", "--abort"]);
        }
        if root.join(".git/rebase-merge").exists() || root.join(".git/rebase-apply").exists() {
            let _ = run(root, &["rebase", "--abort"]);
        }
    }
    result.map(|_| ())
}

/// Link para abrir a nota no Obsidian (os plugins rodam lá).
pub fn obsidian_link(absolute: &Path) -> String {
    let mut encoded = String::new();
    for byte in absolute.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => encoded.push(*byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    format!("obsidian://open?path={encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-git-{}", super::super::store::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn at() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap().and_hms_opt(14, 5, 9).unwrap()
    }

    #[test]
    fn segue_as_configuracoes_do_obsidian_git() {
        let root = temp();
        assert_eq!(settings(&root), Settings::default());
        std::fs::create_dir_all(root.join(".obsidian/plugins/obsidian-git")).unwrap();
        std::fs::write(root.join(".obsidian/community-plugins.json"), r#"["dataview","obsidian-git"]"#).unwrap();
        std::fs::write(root.join(".obsidian/plugins/obsidian-git/data.json"), r#"{"autoSaveInterval":10,"autoCommitMessage":"backup {{date}} ({{numFiles}})","commitDateFormat":"DD/MM/YYYY HH:mm","autoPullOnBoot":true,"disablePush":false,"pullBeforePush":true,"syncMethod":"rebase"}"#).unwrap();
        let s = settings(&root);
        assert!(s.plugin && s.pull_on_open && s.rebase && s.push);
        assert_eq!(s.interval, 10);
        assert_eq!(commit_message(&s, at(), 3), "backup 21/11/2026 14:05 (3)");
        assert_eq!(chrono_format("YYYY-MM-DD HH:mm:ss"), "%Y-%m-%d %H:%M:%S");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn erros_sem_credenciais_e_link_do_obsidian() {
        let error = clean_error("fatal: unable to access 'https://ghp_SEGREDO@github.com/eu/notas.git/': Could not resolve host");
        assert!(!error.contains("ghp_SEGREDO") && error.contains("github.com/eu/notas.git"), "{error}");
        assert_eq!(obsidian_link(Path::new("/home/eu/Cofre/Ideias de RPG.md")), "obsidian://open?path=/home/eu/Cofre/Ideias%20de%20RPG.md");
    }

    /// Repositório real com remoto local: commit, puxar e enviar, e conflito desfeito sem perder nada.
    #[test]
    fn sincroniza_com_remoto_e_desfaz_conflito() {
        let base = temp();
        let remote = base.join("remoto.git");
        let (vault, other) = (base.join("cofre"), base.join("outro"));
        let quiet = |dir: &Path, args: &[&str]| assert!(git(dir).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success(), "{args:?}");
        quiet(&base, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        for dir in [&vault, &other] {
            quiet(&base, &["clone", "-q", remote.to_str().unwrap(), dir.to_str().unwrap()]);
            quiet(dir, &["config", "user.email", "teste@ayla.local"]);
            quiet(dir, &["config", "user.name", "Teste"]);
            quiet(dir, &["checkout", "-q", "-b", "main"]);
        }
        std::fs::write(vault.join("Nota.md"), "um\n").unwrap();
        let first = sync(&vault, &Settings::default(), at()).unwrap();
        assert_eq!((first.committed, first.pushed), (1, true), "{}", first.message);
        assert_eq!(status(&vault).changes, 0);
        // Outro computador muda a mesma linha; aqui também: conflito.
        quiet(&other, &["pull", "-q", "origin", "main"]);
        std::fs::write(other.join("Nota.md"), "dois\n").unwrap();
        quiet(&other, &["commit", "-q", "-am", "outro"]);
        quiet(&other, &["push", "-q", "origin", "main"]);
        std::fs::write(vault.join("Nota.md"), "três\n").unwrap();
        let conflict = sync(&vault, &Settings::default(), at()).unwrap();
        assert!(!conflict.pushed && conflict.message.contains("conflito"), "{}", conflict.message);
        assert_eq!(std::fs::read_to_string(vault.join("Nota.md")).unwrap(), "três\n", "nada se perde");
        assert!(!vault.join(".git/MERGE_HEAD").exists(), "merge desfeito");
        let _ = std::fs::remove_dir_all(&base);
    }
}
