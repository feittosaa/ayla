//! Hand · dotfiles: sincroniza o repositório de dotfiles (o git em que a configuração do Hyprland
//! mora) com o remoto já configurado. A pessoa escolhe os arquivos que entram; a Ayla faz o
//! commit, traz o que mudou lá (`pull --rebase`) e envia (`push`). Pela chave SSH da pessoa, sem
//! pedir senha no terminal: se a chave pedir, avisa em vez de travar.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repositório dos dotfiles: `AYLA_DOTFILES_DIR` (testes), o git onde a configuração do Hyprland
/// está de verdade (links do stow) ou `~/dotfiles`/`~/.dotfiles`.
pub fn achar() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("AYLA_DOTFILES_DIR") { return topo(Path::new(&dir)); }
    let home = super::hand::home();
    let hypr = std::fs::canonicalize(home.join(".config/hypr/hyprland.conf")).ok().and_then(|p| p.parent().map(Path::to_path_buf));
    hypr.into_iter().chain([home.join("dotfiles"), home.join(".dotfiles")]).find_map(|d| topo(&d)).filter(|t| t != &home)
}

fn topo(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    Some(PathBuf::from(out.trim()))
}

/// Roda o git sem prompts (nem de senha nem de SSH).
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args)
        .env("GIT_TERMINAL_PROMPT", "0").env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=15").env("LC_ALL", "C")
        .stdin(std::process::Stdio::null()).output().map_err(|_| "O git não está instalado".to_string())?;
    if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).into_owned()) }
    else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Mudanca {
    pub caminho: String,
    /// "modificado", "novo", "apagado", "renomeado", "fora" (não rastreado).
    pub estado: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Estado {
    pub pasta: String,
    pub ramo: String,
    pub remoto: Option<String>,
    pub endereco: Option<String>,
    /// Commits locais ainda não enviados e do remoto ainda não trazidos (pelo último fetch).
    pub a_enviar: u32,
    pub a_trazer: u32,
    pub mudancas: Vec<Mudanca>,
}

/// Lê o `git status --porcelain -z`.
pub fn ler_status(texto: &str) -> Vec<Mudanca> {
    let mut out = Vec::new();
    let mut parts = texto.split('\0').filter(|p| !p.is_empty());
    while let Some(entry) = parts.next() {
        if entry.len() < 4 { continue; }
        let (code, path) = entry.split_at(3);
        let code = code.trim();
        let estado = match code {
            "??" => "fora",
            c if c.contains('R') => { let _ = parts.next(); "renomeado" }
            c if c.contains('D') => "apagado",
            c if c.contains('A') => "novo",
            _ => "modificado",
        };
        out.push(Mudanca { caminho: path.to_string(), estado: estado.into() });
    }
    out
}

pub fn estado(dir: &Path) -> Result<Estado, String> {
    let ramo = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string();
    let remoto = git(dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok().map(|s| s.trim().to_string());
    let endereco = remoto.as_ref().and_then(|r| r.split('/').next()).and_then(|name| git(dir, &["remote", "get-url", name]).ok()).map(|s| s.trim().to_string());
    let (mut a_enviar, mut a_trazer) = (0, 0);
    if remoto.is_some() {
        if let Ok(out) = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]) {
            let mut n = out.split_whitespace().map(|x| x.parse::<u32>().unwrap_or(0));
            a_enviar = n.next().unwrap_or(0);
            a_trazer = n.next().unwrap_or(0);
        }
    }
    let mudancas = ler_status(&git(dir, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?);
    Ok(Estado { pasta: dir.to_string_lossy().into_owned(), ramo, remoto, endereco, a_enviar, a_trazer, mudancas })
}

/// Texto de um arquivo da lista (o atual; o apagado, como estava no último commit) e, se mudou,
/// a diferença. Binários e arquivos grandes vêm cortados.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Conteudo {
    pub texto: String,
    pub diferenca: Option<String>,
    pub binario: bool,
    pub cortado: bool,
}

const LIMITE: usize = 200_000;

pub fn conteudo(dir: &Path, caminho: &str) -> Result<Conteudo, String> {
    let atual = estado(dir)?;
    let m = atual.mudancas.iter().find(|m| m.caminho == caminho).ok_or("Esse arquivo não está na lista")?;
    let bytes = if m.estado == "apagado" {
        let out = Command::new("git").arg("-C").arg(dir).args(["show", &format!("HEAD:{caminho}")]).output().map_err(|_| "O git não está instalado")?;
        if !out.status.success() { return Err("Não achei a versão anterior do arquivo".into()); }
        out.stdout
    } else {
        let path = dir.join(caminho);
        if path.is_dir() { return Ok(Conteudo { texto: "(pasta)".into(), diferenca: None, binario: false, cortado: false }); }
        std::fs::read(&path).map_err(|_| "Não foi possível ler o arquivo")?
    };
    let binario = bytes.iter().take(8000).any(|b| *b == 0);
    let cortado = bytes.len() > LIMITE;
    let texto = if binario { String::new() } else { String::from_utf8_lossy(&bytes[..bytes.len().min(LIMITE)]).into_owned() };
    let diferenca = if matches!(m.estado.as_str(), "modificado" | "renomeado") && !binario {
        git(dir, &["diff", "HEAD", "--", caminho]).ok().map(|d| d.chars().take(LIMITE).collect())
    } else { None };
    Ok(Conteudo { texto, diferenca, binario, cortado })
}

/// Começo do texto de um arquivo (para a Ayla explicar o que ele faz).
pub fn amostra(dir: &Path, caminho: &str, estado: &str) -> String {
    let bytes = if estado == "apagado" {
        Command::new("git").arg("-C").arg(dir).args(["show", &format!("HEAD:{caminho}")]).output().map(|o| o.stdout).unwrap_or_default()
    } else { std::fs::read(dir.join(caminho)).unwrap_or_default() };
    if bytes.iter().take(4000).any(|b| *b == 0) { return "(arquivo binário)".into(); }
    String::from_utf8_lossy(&bytes[..bytes.len().min(1500)]).into_owned()
}

/// Parece ter dado que não deveria ir para o repositório (chave, senha, token…)?
pub fn sensivel(dir: &Path, caminho: &str, estado: &str) -> Option<String> {
    let nome = caminho.to_lowercase();
    if ["id_rsa", "id_ed25519", ".pem", ".key", ".env", "token", "secret", "segredo", "password", "senha", ".netrc", "credentials"].iter().any(|k| nome.contains(k)) {
        return Some("o nome parece de chave, senha ou token".into());
    }
    if estado == "apagado" { return None; }
    let text = std::fs::read(dir.join(caminho)).ok().map(|b| String::from_utf8_lossy(&b[..b.len().min(LIMITE)]).into_owned()).unwrap_or_default();
    if text.contains("PRIVATE KEY-----") { return Some("tem uma chave privada".into()); }
    super::policy::certain_reason(&text).filter(|r| !r.contains("e-mail")).map(|r| format!("tem {r}"))
}

pub fn explicacao_pedido(itens: &[(String, String, String)]) -> Vec<serde_json::Value> {
    let lista: Vec<serde_json::Value> = itens.iter().map(|(caminho, estado, amostra)| serde_json::json!({"arquivo": caminho, "mudanca": estado, "comeco": amostra})).collect();
    vec![
        serde_json::json!({"role":"system","content":"Você explica arquivos de configuração (dotfiles) de um Linux com Hyprland para uma pessoa, em português do Brasil. Para cada arquivo, diga em uma frase curta (até 25 palavras) o que ele configura ou faz no sistema, pelo caminho e pelo começo do texto (\"Atalhos de teclado pessoais do Hyprland\", \"Cópia de segurança do .bashrc\"). Não invente; se não der para saber, diga o que dá pelo nome. Responda só o JSON {\"arquivos\":[{\"arquivo\":\"…\",\"explicacao\":\"…\"}]}."}),
        serde_json::json!({"role":"user","content": serde_json::to_string(&lista).unwrap_or_default()}),
    ]
}

pub fn explicacao_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{"arquivos":{"type":"array","items":{"type":"object","properties":{"arquivo":{"type":"string"},"explicacao":{"type":"string"}},"required":["arquivo","explicacao"]}}},"required":["arquivos"]})
}

pub fn ler_explicacoes(raw: &str) -> Vec<(String, String)> {
    let start = raw.find('{').unwrap_or(0);
    let v: serde_json::Value = serde_json::from_str(&raw[start..]).unwrap_or(serde_json::Value::Null);
    v["arquivos"].as_array().into_iter().flatten().filter_map(|a| {
        let (k, e) = (a["arquivo"].as_str()?, a["explicacao"].as_str()?.trim());
        (!e.is_empty()).then(|| (k.to_string(), e.chars().take(240).collect()))
    }).collect()
}

/// Mensagem clara para os erros mais comuns do git/ssh.
pub fn explicar(erro: &str) -> String {
    let e = erro.to_lowercase();
    if e.contains("permission denied (publickey") || e.contains("passphrase") || e.contains("host key verification") {
        "O GitHub não aceitou a chave SSH sem senha (a chave pede senha e não há agente com ela carregada, ou o servidor não é conhecido). Rode `ssh-add` no terminal e tente de novo.".into()
    } else if e.contains("could not resolve host") || e.contains("connection timed out") || e.contains("network is unreachable") {
        "Sem conexão com o remoto agora. Nada foi enviado; o commit ficou guardado aqui.".into()
    } else if e.contains("conflict") {
        "As mudanças daqui e as do remoto mexem nas mesmas linhas. Desfiz a tentativa de juntar; o commit ficou guardado aqui. Resolva no terminal (git pull --rebase).".into()
    } else {
        format!("O git respondeu: {}", erro.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(erro).trim())
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Resultado {
    pub commit: bool,
    pub trazidos: u32,
    pub enviados: u32,
}

/// Commit de tudo o que mudou (os mudados, os novos e os apagados que a página mostrou),
/// `pull --rebase --autostash` e `push`. `esperados`: a lista que a pessoa viu; se o repositório
/// mudou desde então, nada é enviado e a página recarrega.
pub fn sincronizar(dir: &Path, esperados: &[String], mensagem: &str) -> Result<Resultado, String> {
    let atual = estado(dir)?;
    if atual.remoto.is_none() { return Err("Este ramo não tem um remoto configurado (git push -u).".into()); }
    let mut agora: Vec<&str> = atual.mudancas.iter().map(|m| m.caminho.as_str()).collect();
    let mut vistos: Vec<&str> = esperados.iter().map(String::as_str).collect();
    agora.sort_unstable(); vistos.sort_unstable();
    if agora != vistos { return Err("Os arquivos mudaram desde que a lista abriu. Confira a lista de novo e sincronize.".into()); }
    let mut commit = false;
    if !atual.mudancas.is_empty() {
        let mensagem = mensagem.trim();
        if mensagem.is_empty() { return Err("Escreva uma mensagem para o commit".into()); }
        git(dir, &["add", "-A"]).map_err(|e| explicar(&e))?;
        git(dir, &["commit", "-m", mensagem]).map_err(|e| explicar(&e))?;
        commit = true;
    }
    // O remoto como estava (pelo último fetch): o que chegar depois dele foi trazido agora.
    let antes = git(dir, &["rev-parse", "@{u}"])?.trim().to_string();
    if let Err(e) = git(dir, &["pull", "--rebase", "--autostash"]) {
        if dir.join(".git/rebase-merge").exists() || dir.join(".git/rebase-apply").exists() { let _ = git(dir, &["rebase", "--abort"]); }
        return Err(explicar(&e));
    }
    let trazidos = git(dir, &["rev-list", "--count", &format!("{antes}..@{{u}}")]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let enviados = git(dir, &["rev-list", "--count", "@{u}..HEAD"]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    if enviados > 0 { git(dir, &["push"]).map_err(|e| explicar(&e))?; }
    Ok(Resultado { commit, trazidos, enviados })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) { assert!(Command::new("git").arg("-C").arg(dir).args(args).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap().status.success(), "{args:?}"); }

    #[test]
    fn status_do_git() {
        let list = ler_status(" M hypr/a.conf\0?? fish/x.bak\0A  novo.conf\0 D velho.conf\0R  b.conf\0a.conf\0");
        let estados: Vec<(&str, &str)> = list.iter().map(|m| (m.caminho.as_str(), m.estado.as_str())).collect();
        assert_eq!(estados, vec![("hypr/a.conf", "modificado"), ("fish/x.bak", "fora"), ("novo.conf", "novo"), ("velho.conf", "apagado"), ("b.conf", "renomeado")]);
        assert!(explicar("git@github.com: Permission denied (publickey).").contains("ssh-add"));
        assert_eq!(ler_explicacoes("{\"arquivos\":[{\"arquivo\":\"a.conf\",\"explicacao\":\"Atalhos\"},{\"arquivo\":\"b\",\"explicacao\":\"\"}]}"), vec![("a.conf".to_string(), "Atalhos".to_string())]);
    }

    #[test]
    fn sincroniza_com_um_remoto_local() {
        let root = std::env::temp_dir().join(format!("ayla-dotfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let remote = root.join("remoto.git");
        run(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
        let (a, b) = (root.join("a"), root.join("b"));
        run(&root, &["clone", "-q", remote.to_str().unwrap(), a.to_str().unwrap()]);
        for d in [&a] { run(d, &["config", "user.name", "t"]); run(d, &["config", "user.email", "t@t"]); }
        std::fs::write(a.join("hypr.conf"), "x\n").unwrap();
        run(&a, &["add", "."]); run(&a, &["commit", "-qm", "inicio"]); run(&a, &["push", "-q", "-u", "origin", "main"]);
        run(&root, &["clone", "-q", remote.to_str().unwrap(), b.to_str().unwrap()]);
        run(&b, &["config", "user.name", "t"]); run(&b, &["config", "user.email", "t@t"]);
        // Mudança no outro computador (b), já enviada.
        std::fs::write(b.join("fish.conf"), "y\n").unwrap();
        run(&b, &["add", "."]); run(&b, &["commit", "-qm", "outro pc"]); run(&b, &["push", "-q"]);
        // Aqui (a): uma mudança escolhida e um arquivo novo que fica de fora.
        std::fs::write(a.join("hypr.conf"), "x2\n").unwrap();
        std::fs::write(a.join("segredo.bak"), "s\n").unwrap();
        let e = estado(&a).unwrap();
        assert_eq!(e.mudancas.len(), 2);
        assert!(sincronizar(&a, &["hypr.conf".into()], "m").is_err(), "lista diferente da vista: não envia");
        let c = conteudo(&a, "hypr.conf").unwrap();
        assert_eq!(c.texto, "x2\n");
        assert!(c.diferenca.unwrap().contains("+x2"));
        assert!(sensivel(&a, "segredo.bak", "fora").is_some(), "nome com segredo");
        let r = sincronizar(&a, &["hypr.conf".into(), "segredo.bak".into()], "Atualização pela Ayla").unwrap();
        assert_eq!(r, Resultado { commit: true, trazidos: 1, enviados: 1 });
        assert!(a.join("fish.conf").exists(), "trouxe o do outro pc");
        let depois = estado(&a).unwrap();
        assert!(depois.mudancas.is_empty(), "foi tudo");
        assert_eq!((depois.a_enviar, depois.a_trazer), (0, 0));
        // Apagado: o texto vem do último commit.
        std::fs::remove_file(a.join("fish.conf")).unwrap();
        assert_eq!(conteudo(&a, "fish.conf").unwrap().texto, "y\n");
        run(&b, &["pull", "-q"]);
        assert_eq!(std::fs::read_to_string(b.join("hypr.conf")).unwrap(), "x2\n", "chegou no remoto");
        assert!(b.join("segredo.bak").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
