//! Bases de dados públicas curadas. Cada base do catálogo é fixada num commit da fonte oficial
//! (pacote com SHA-256 conferido), montada como um SQLite próprio em `<dados>/bases/<id>.db` e
//! consultada neste computador, somente leitura: o modelo local escreve um SELECT a partir do
//! esquema e do guia da base, e o núcleo valida e executa.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

/// Uma base embutida no catálogo da Ayla (dados fixos no código).
pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub publisher: &'static str,
    pub description: &'static str,
    pub license: &'static str,
    /// Repositório no GitHub (`dono/nome`) e commit fixado.
    pub repository: &'static str,
    pub commit: &'static str,
    /// Tamanho e SHA-256 do pacote `.tar.gz` desse commit.
    pub bytes: u64,
    pub sha256: &'static str,
    /// Pasta das migrações Flyway (SQLite) dentro do repositório.
    pub migrations: &'static str,
    /// Consulta que devolve a versão dos dados (mostrada na fonte da resposta).
    pub version_query: &'static str,
    /// Termos (sem acento, minúsculos) que indicam uma pergunta para esta base.
    pub keywords: &'static [&'static str],
    /// Guia curado: o que cada tabela importante guarda e como ligar umas às outras.
    pub guide: &'static str,
    /// Exemplos conferidos (pergunta, SQL), que ensinam o modelo local a consultar a base.
    pub examples: &'static [(&'static str, &'static str)],
}

const CATALOG: &[Entry] = &[Entry {
    id: "receita-rtc",
    name: "Tabelas da Reforma Tributária do Consumo",
    publisher: "Receita Federal",
    description: "NCM, NBS, classificações tributárias (cClassTrib), situações tributárias, alíquotas de IBS, CBS e Imposto Seletivo, reduções, anexos da LC 214/2025, fundamentação legal, lista de serviços da LC 116 e municípios, da calculadora oficial do regime regular.",
    license: "MIT",
    repository: "ReceitaFederal/calculadora-rtc-regime-regular",
    commit: "f1eebd7ce3157423b1a6aaca7dc1d6c3395452c4",
    bytes: 1_835_306,
    sha256: "0f299a18bf0c20f99dc97bf3adc72235d86b6e52b4400083cb9984eb086718d6",
    migrations: "flyway/sql",
    version_query: "SELECT VRBD_VERSAO_BASE_DADO FROM VERSAO_BASE_DADO ORDER BY VRBD_ID DESC LIMIT 1",
    keywords: &[
        "ncm", "nbs", "cclasstrib", "classificacao tributaria", "classificacoes tributarias", "situacao tributaria",
        "aliquota", "aliquotas", "ibs", "cbs", "imposto seletivo", "reforma tributaria", "lc 214", "lc 116",
        "lei complementar 214", "fundamentacao legal", "ad valorem", "ad rem", "credito presumido", "reducao de aliquota",
        "cesta basica", "tributacao do consumo", "imposto sobre bens e servicos", "contribuicao sobre bens e servicos",
    ],
    guide: "Guia da base (Receita Federal, reforma tributária do consumo):
- NCM(NCM_CD, NCM_DESCRICAO): produtos. Códigos com 2, 4, 6 ou 8 dígitos (capítulo, posição, subposição, item), sem pontos.
- NBS(NBS_CD, NBS_DESCRICAO, NBS_LC_116): serviços.
- CLASSIFICACAO_TRIBUTARIA(CLTR_ID, CLTR_CD = cClassTrib de 6 dígitos, CLTR_DESCRICAO, CLTR_TIPO_ALIQUOTA, CLTR_SITR_ID → SITUACAO_TRIBUTARIA.SITR_ID). SITUACAO_TRIBUTARIA(SITR_CD = CST de 3 dígitos, SITR_DESCRICAO).
- NCM_APLICAVEL(NCMA_NCM_CD, NCMA_CLTR_ID, NCMA_ANXO_ID) liga produtos a classificações; NCMA_NCM_CD é um PREFIXO: um NCM de 8 dígitos está coberto quando '<ncm>' LIKE NCMA_NCM_CD || '%'. NBS_APLICAVEL faz o mesmo para serviços.
- ANEXO(ANXO_ID, ANXO_NUMERO em algarismos romanos, ANXO_TEXTO_ITEM): anexos da LC 214/2025 (ex.: I = cesta básica nacional).
- TRIBUTO(TBTO_ID, TBTO_NOME, TBTO_SIGLA: IS, CBS, IBS…). ALIQUOTA_REFERENCIA(ALRE_TBTO_ID, ALRE_VALOR em %, vigência). PERCENTUAL_REDUCAO(PERE_CLTR_ID, PERE_TBTO_ID, PERE_VALOR em % de redução).
- ALIQUOTA_AD_VALOREM(AADV_ID, AADV_TBTO_ID, AADV_VALOR em %) e ALIQUOTA_AD_VALOREM_PRODUTO(AAVP_NCM_CD, AAVP_AADV_ID): alíquotas específicas do Imposto Seletivo por NCM.
- FUNDAMENTACAO_LEGAL(FDLG_ID, FDLG_TEXTO, FDLG_TEXTO_CURTO, FDLG_REFERENCIA_NORMATIVA) ligada por FUNDAMENTACAO_CLASSIFICACAO(FDCL_CLTR_ID, FDCL_FDLG_ID).
- LISTA_SERVICOS_LC_116(LSLC_CD, LSLC_DESCRICAO). MUNICIPIO(MUNI_CD = código IBGE, MUNI_NOME, MUNI_UF_CD → UF.UF_CD). UF(UF_SIGLA, UF_NOME).
- Vigência: colunas *_INICIO_VIGENCIA e *_FIM_VIGENCIA (texto AAAA-MM-DD; fim vazio ou 9999-12-31 = vigente). O mesmo CLTR_CD pode aparecer em mais de uma linha (conjuntos diferentes); traga a descrição para distinguir.
- Valor vazio numa alíquota quer dizer que a base ainda não define esse valor.",
    examples: &[
        ("Qual o NCM da cerveja?", "SELECT NCM_CD, NCM_DESCRICAO FROM NCM WHERE NCM_DESCRICAO_BUSCA LIKE '%cerveja%' LIMIT 30"),
        ("Qual a alíquota de referência da CBS?", "SELECT t.TBTO_SIGLA, r.ALRE_VALOR, r.ALRE_INICIO_VIGENCIA, r.ALRE_FIM_VIGENCIA FROM ALIQUOTA_REFERENCIA r JOIN TRIBUTO t ON t.TBTO_ID = r.ALRE_TBTO_ID WHERE t.TBTO_SIGLA = 'CBS' LIMIT 30"),
        ("Qual a classificação tributária e o anexo do NCM 0901.21.00?", "SELECT a.NCMA_NCM_CD, c.CLTR_CD, c.CLTR_DESCRICAO, x.ANXO_NUMERO, x.ANXO_TEXTO_ITEM FROM NCM_APLICAVEL a JOIN CLASSIFICACAO_TRIBUTARIA c ON c.CLTR_ID = a.NCMA_CLTR_ID LEFT JOIN ANEXO x ON x.ANXO_ID = a.NCMA_ANXO_ID WHERE '09012100' LIKE a.NCMA_NCM_CD || '%' LIMIT 30"),
        ("Qual a redução de alíquota do cClassTrib 200003?", "SELECT c.CLTR_CD, c.CLTR_DESCRICAO, t.TBTO_SIGLA, p.PERE_VALOR FROM CLASSIFICACAO_TRIBUTARIA c JOIN PERCENTUAL_REDUCAO p ON p.PERE_CLTR_ID = c.CLTR_ID JOIN TRIBUTO t ON t.TBTO_ID = p.PERE_TBTO_ID WHERE c.CLTR_CD = '200003' LIMIT 30"),
        ("Qual a fundamentação legal do cClassTrib 000001?", "SELECT c.CLTR_CD, c.CLTR_DESCRICAO, f.FDLG_TEXTO_CURTO, f.FDLG_TEXTO FROM CLASSIFICACAO_TRIBUTARIA c JOIN FUNDAMENTACAO_CLASSIFICACAO fc ON fc.FDCL_CLTR_ID = c.CLTR_ID JOIN FUNDAMENTACAO_LEGAL f ON f.FDLG_ID = fc.FDCL_FDLG_ID WHERE c.CLTR_CD = '000001' LIMIT 30"),
        ("Qual a alíquota do Imposto Seletivo sobre charutos?", "SELECT p.AAVP_NCM_CD, n.NCM_DESCRICAO, a.AADV_VALOR, p.AAVP_INICIO_VIGENCIA, p.AAVP_FIM_VIGENCIA FROM ALIQUOTA_AD_VALOREM_PRODUTO p JOIN ALIQUOTA_AD_VALOREM a ON a.AADV_ID = p.AAVP_AADV_ID JOIN NCM n ON n.NCM_CD = p.AAVP_NCM_CD WHERE n.NCM_DESCRICAO_BUSCA LIKE '%charuto%' LIMIT 30"),
    ],
}];

/// Como a base é montada a partir do pacote da fonte.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "lowercase")]
pub enum Format {
    /// Migrações Flyway para SQLite nesta pasta do repositório.
    Flyway { pasta: String },
    /// Arquivos CSV (caminhos no repositório); cada um vira uma tabela.
    Csv { arquivos: Vec<String> },
}

/// Uma base do catálogo: embutida pela Ayla ou adicionada por link e aprovada pela pessoa.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Base {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub description: String,
    pub license: String,
    pub repository: String,
    pub commit: String,
    pub bytes: u64,
    pub sha256: String,
    pub format: Format,
    #[serde(default)]
    pub version_query: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub guide: String,
    #[serde(default)]
    pub examples: Vec<(String, String)>,
    /// "ayla" (embutida) ou "voce" (link validado e aprovado).
    #[serde(default)]
    pub origin: String,
}

impl Entry {
    pub fn base(&self) -> Base {
        Base {
            id: self.id.into(), name: self.name.into(), publisher: self.publisher.into(), description: self.description.into(),
            license: self.license.into(), repository: self.repository.into(), commit: self.commit.into(), bytes: self.bytes,
            sha256: self.sha256.into(), format: Format::Flyway { pasta: self.migrations.into() },
            version_query: Some(self.version_query.into()), keywords: self.keywords.iter().map(|k| k.to_string()).collect(),
            guide: self.guide.into(), examples: self.examples.iter().map(|(q, s)| (q.to_string(), s.to_string())).collect(), origin: "ayla".into(),
        }
    }
}

fn user_catalog_path(root: &Path) -> PathBuf {
    dir(root).join("catalogo.json")
}

/// Bases adicionadas por link e aprovadas pela pessoa.
pub fn user_catalog(root: &Path) -> Vec<Base> {
    fs::read(user_catalog_path(root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_user_catalog(root: &Path, list: &[Base]) -> Result<(), String> {
    fs::create_dir_all(dir(root)).map_err(|_| "Não foi possível criar a pasta das bases")?;
    let path = user_catalog_path(root);
    let temporary = path.with_extension("json.part");
    fs::write(&temporary, serde_json::to_vec_pretty(list).map_err(|_| "Falha ao gravar o catálogo")?)
        .and_then(|()| fs::rename(&temporary, &path)).map_err(|_| "Não foi possível gravar o catálogo".into())
}

/// Catálogo completo: as bases da Ayla e as aprovadas pela pessoa.
pub fn catalog(root: &Path) -> Vec<Base> {
    let mut out: Vec<Base> = CATALOG.iter().map(Entry::base).collect();
    for base in user_catalog(root) {
        if !out.iter().any(|b| b.id == base.id) {
            out.push(base);
        }
    }
    out
}

pub fn find(root: &Path, id: &str) -> Option<Base> {
    catalog(root).into_iter().find(|b| b.id == id)
}

impl Base {
    pub fn archive_url(&self) -> String {
        format!("https://codeload.github.com/{}/tar.gz/{}", self.repository, self.commit)
    }
    pub fn source_url(&self) -> String {
        format!("https://github.com/{}/tree/{}", self.repository, self.commit)
    }
}

/// Base instalada neste computador.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Installed {
    pub commit: String,
    pub version: Option<String>,
    pub bytes: u64,
    pub installed_at: i64,
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("bases")
}
pub fn db_path(root: &Path, id: &str) -> PathBuf {
    dir(root).join(format!("{id}.db"))
}
fn meta_path(root: &Path, id: &str) -> PathBuf {
    dir(root).join(format!("{id}.json"))
}

pub fn installed(root: &Path, id: &str) -> Option<Installed> {
    if !db_path(root, id).is_file() {
        return None;
    }
    serde_json::from_slice(&fs::read(meta_path(root, id)).ok()?).ok()
}

/// Põe no lugar a base recém-montada (`part`) e grava o registro da instalação.
pub fn finish(root: &Path, entry: &Base, part: &Path, version: Option<String>) -> Result<Installed, String> {
    let installed_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let installed = Installed { commit: entry.commit.clone(), version, bytes: fs::metadata(part).map_or(0, |m| m.len()), installed_at };
    let meta = meta_path(root, &entry.id);
    let temporary = meta.with_extension("json.part");
    fs::write(&temporary, serde_json::to_vec_pretty(&installed).map_err(|_| "Falha ao registrar a base")?)
        .and_then(|()| fs::rename(part, db_path(root, &entry.id)))
        .and_then(|()| fs::rename(&temporary, &meta))
        .map_err(|_| "Não foi possível guardar a base")?;
    Ok(installed)
}

/// Remove a base (banco e registro); uma base adicionada por link sai também do catálogo.
/// Os dados vêm de fora e podem ser baixados de novo.
pub fn remove(root: &Path, id: &str) -> Result<(), String> {
    let mut list = user_catalog(root);
    if list.iter().any(|b| b.id == id) {
        list.retain(|b| b.id != id);
        save_user_catalog(root, &list)?;
    }
    for path in [db_path(root, id), meta_path(root, id)] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Não foi possível remover a base".into()),
        }
    }
    Ok(())
}

const MAX_SCRIPT: u64 = 48 << 20;
const MAX_SCRIPTS: u64 = 160 << 20;

/// Lê do pacote só os scripts `.sql` da pasta de migrações (caminho relativo a ela).
fn scripts(archive: &Path, migrations: &str) -> Result<BTreeMap<String, String>, String> {
    let file = fs::File::open(archive).map_err(|_| "Pacote da base indisponível")?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let prefix = format!("{}/", migrations.trim_matches('/'));
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    for item in tar.entries().map_err(|_| "Pacote da base inválido")? {
        let mut item = item.map_err(|_| "Pacote da base inválido")?;
        if !item.header().entry_type().is_file() {
            continue;
        }
        let path = item.path().map_err(|_| "Pacote da base inválido")?.to_string_lossy().into_owned();
        // O GitHub põe tudo numa pasta `<repositório>-<commit>/`.
        let Some(inside) = path.split_once('/').map(|(_, rest)| rest) else { continue };
        let Some(relative) = inside.strip_prefix(&prefix) else { continue };
        if !relative.to_ascii_lowercase().ends_with(".sql") || relative.split('/').any(|p| p.is_empty() || p == ".." || p.starts_with('.')) {
            continue;
        }
        let size = item.header().size().unwrap_or(u64::MAX);
        total += size;
        if size > MAX_SCRIPT || total > MAX_SCRIPTS {
            return Err("A base passa do tamanho esperado".into());
        }
        let mut text = String::new();
        item.read_to_string(&mut text).map_err(|_| "Script da base inválido (precisa estar em UTF-8)")?;
        out.insert(relative.to_string(), text);
    }
    Ok(out)
}

/// Lê do pacote os arquivos pedidos (caminhos no repositório), em bytes.
fn files(archive: &Path, wanted: &[String]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let file = fs::File::open(archive).map_err(|_| "Pacote da base indisponível")?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut out = Vec::new();
    let mut total = 0u64;
    for item in tar.entries().map_err(|_| "Pacote da base inválido")? {
        let mut item = item.map_err(|_| "Pacote da base inválido")?;
        if !item.header().entry_type().is_file() {
            continue;
        }
        let path = item.path().map_err(|_| "Pacote da base inválido")?.to_string_lossy().into_owned();
        let Some(inside) = path.split_once('/').map(|(_, rest)| rest.to_string()) else { continue };
        if !wanted.iter().any(|w| *w == inside) {
            continue;
        }
        let size = item.header().size().unwrap_or(u64::MAX);
        total += size;
        if size > MAX_SCRIPT || total > MAX_SCRIPTS {
            return Err("A base passa do tamanho esperado".into());
        }
        let mut bytes = Vec::new();
        item.read_to_end(&mut bytes).map_err(|_| "Arquivo da base inválido")?;
        out.push((inside, bytes));
    }
    if out.len() != wanted.len() {
        return Err("O pacote não tem todos os arquivos da base".into());
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Importação de CSV: cada arquivo vira uma tabela de colunas de texto.
pub mod csv {
    use rusqlite::Connection;

    /// Texto em UTF-8 (sem BOM) ou, se não for, em Latin-1/Windows-1252.
    pub fn decode(bytes: &[u8]) -> String {
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        match std::str::from_utf8(bytes) {
            Ok(text) => text.to_string(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        }
    }

    /// Separador mais provável pela primeira linha: vírgula, ponto e vírgula ou tabulação.
    pub fn delimiter(text: &str) -> char {
        let first = text.lines().next().unwrap_or("");
        [',', ';', '\t'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',')
    }

    /// Linhas e campos, com aspas duplas (e `""` dentro delas) e quebras de linha entre aspas.
    pub fn parse(text: &str, delimiter: char) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        let mut row = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if quoted {
                if c == '"' {
                    if chars.peek() == Some(&'"') { field.push('"'); chars.next(); } else { quoted = false; }
                } else {
                    field.push(c);
                }
            } else if c == '"' && field.is_empty() {
                quoted = true;
            } else if c == delimiter {
                row.push(std::mem::take(&mut field));
            } else if c == '\n' || c == '\r' {
                if c == '\r' && chars.peek() == Some(&'\n') { chars.next(); }
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) { rows.push(std::mem::take(&mut row)); } else { row.clear(); }
            } else {
                field.push(c);
            }
        }
        if !field.is_empty() || !row.is_empty() {
            row.push(field);
            rows.push(row);
        }
        rows
    }

    /// Nome SQL seguro: letras e números ASCII em maiúsculas, com `_`; nunca vazio nem começando com número.
    pub fn name(raw: &str) -> String {
        let folded = super::fold(raw);
        let mut out: String = folded.trim().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
        out = out.trim_matches('_').to_string();
        while out.contains("__") { out = out.replace("__", "_"); }
        if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) { out = format!("T_{out}"); }
        out
    }

    pub fn import(db: &Connection, files: &[(String, Vec<u8>)]) -> Result<(), String> {
        let mut tables = Vec::new();
        db.execute_batch("BEGIN").map_err(|_| "Falha ao importar")?;
        for (path, bytes) in files {
            let text = decode(bytes);
            let rows = parse(&text, delimiter(&text));
            let Some((header, data)) = rows.split_first() else { continue };
            let stem = path.rsplit('/').next().unwrap_or(path).rsplit_once('.').map_or(path.as_str(), |(s, _)| s);
            let mut table = name(stem);
            while tables.contains(&table) { table.push('_'); }
            let mut columns: Vec<String> = Vec::new();
            for (i, raw) in header.iter().enumerate() {
                let mut column = if raw.trim().is_empty() { format!("COLUNA_{}", i + 1) } else { name(raw) };
                while columns.contains(&column) { column.push('_'); }
                columns.push(column);
            }
            let quoted: Vec<String> = columns.iter().map(|c| format!("\"{c}\" TEXT")).collect();
            db.execute_batch(&format!("CREATE TABLE \"{table}\" ({})", quoted.join(", "))).map_err(|e| format!("CSV {path}: {e}"))?;
            let marks = vec!["?"; columns.len()].join(", ");
            let mut insert = db.prepare(&format!("INSERT INTO \"{table}\" VALUES ({marks})")).map_err(|e| format!("CSV {path}: {e}"))?;
            for row in data {
                let values: Vec<Option<&str>> = (0..columns.len()).map(|i| row.get(i).map(String::as_str).filter(|v| !v.is_empty())).collect();
                insert.execute(rusqlite::params_from_iter(values)).map_err(|e| format!("CSV {path}: {e}"))?;
            }
            tables.push(table);
        }
        if tables.is_empty() {
            let _ = db.execute_batch("ROLLBACK");
            return Err("Nenhum CSV com cabeçalho".into());
        }
        db.execute_batch("COMMIT").map_err(|_| "Falha ao importar".into())
    }
}

/// Bases sugeridas por link (GitHub): a Ayla confere a fonte e monta uma cópia para a pessoa aprovar.
pub mod link {
    use super::{Base, Format};
    use sha2::{Digest, Sha256};
    use std::{io::Write, path::Path};

    pub const MAX_ARCHIVE: u64 = 300 << 20;
    /// Licenças abertas reconhecidas (SPDX). Outras exigem atenção da pessoa.
    pub const OPEN: &[&str] = &["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unlicense", "CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0", "ODbL-1.0", "PDDL-1.0", "MPL-2.0", "GPL-3.0", "LGPL-3.0", "AGPL-3.0", "GPL-2.0", "0BSD"];

    #[derive(Debug, Clone, PartialEq)]
    pub struct Link { pub owner: String, pub repo: String, pub reference: Option<String>, pub path: String }

    fn valid_part(part: &str) -> bool {
        !part.is_empty() && part.len() <= 100 && part != "." && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    }

    /// `https://github.com/dono/repo[/tree/<ref>[/pasta]]`. Outros sites ainda não são aceitos.
    pub fn parse(url: &str) -> Result<Link, String> {
        let rest = url.trim().trim_end_matches('/').strip_prefix("https://github.com/").or_else(|| url.trim().strip_prefix("github.com/"))
            .ok_or("Cole um link de repositório do GitHub (https://github.com/dono/repositório)")?;
        let parts: Vec<&str> = rest.split(['?', '#']).next().unwrap_or("").split('/').collect();
        let (owner, repo) = (parts.first().copied().unwrap_or(""), parts.get(1).copied().unwrap_or("").trim_end_matches(".git"));
        if !valid_part(owner) || !valid_part(repo) {
            return Err("Link do GitHub inválido".into());
        }
        let (reference, path) = match parts.get(2) {
            Some(&"tree") | Some(&"blob") => {
                let reference = parts.get(3).filter(|r| valid_part(r)).ok_or("Link do GitHub inválido")?.to_string();
                let path: Vec<&str> = parts[4..].to_vec();
                if path.iter().any(|p| !valid_part(p)) { return Err("Link do GitHub inválido".into()); }
                (Some(reference), path.join("/"))
            }
            None => (None, String::new()),
            _ => return Err("Link do GitHub inválido".into()),
        };
        Ok(Link { owner: owner.into(), repo: repo.into(), reference, path })
    }

    /// Baixa até `MAX_ARCHIVE` bytes; devolve tamanho e SHA-256.
    pub async fn download(client: &reqwest::Client, url: &str, destination: &Path) -> Result<(u64, String), String> {
        use futures_util::StreamExt;
        let response = client.get(url).send().await.map_err(|_| "Não foi possível baixar a fonte")?
            .error_for_status().map_err(|_| "O GitHub não entregou o pacote")?;
        let mut file = std::fs::File::create(destination).map_err(|_| "Não foi possível salvar o pacote")?;
        let mut sha = Sha256::new();
        let mut total = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Download interrompido")?;
            total += chunk.len() as u64;
            if total > MAX_ARCHIVE {
                return Err("A fonte passa de 300 MB; por enquanto a Ayla só guarda bases menores".into());
            }
            sha.update(&chunk);
            file.write_all(&chunk).map_err(|_| "Sem espaço para o pacote")?;
        }
        Ok((total, format!("{:x}", sha.finalize())))
    }

    /// Formato reconhecido dentro de `path` (vazio = repositório todo): migrações Flyway para
    /// SQLite (`B`/`V<n>__*.sql`) ou arquivos CSV (até 40).
    pub fn detect(archive: &Path, path: &str) -> Result<Format, String> {
        let file = std::fs::File::open(archive).map_err(|_| "Pacote indisponível")?;
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let prefix = if path.is_empty() { String::new() } else { format!("{}/", path.trim_matches('/')) };
        let mut migrations: Vec<String> = Vec::new();
        let mut csvs: Vec<String> = Vec::new();
        for item in tar.entries().map_err(|_| "Pacote inválido")? {
            let item = item.map_err(|_| "Pacote inválido")?;
            if !item.header().entry_type().is_file() { continue; }
            let full = item.path().map_err(|_| "Pacote inválido")?.to_string_lossy().into_owned();
            let Some(inside) = full.split_once('/').map(|(_, r)| r.to_string()) else { continue };
            if !inside.starts_with(&prefix) || inside.split('/').any(|p| p.starts_with('.')) { continue; }
            let name = inside.rsplit('/').next().unwrap_or("");
            let lower = name.to_ascii_lowercase();
            let versioned = (name.starts_with('V') || name.starts_with('B')) && name[1..].split_once("__").is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '_'));
            if lower.ends_with(".sql") && versioned {
                migrations.push(inside);
            } else if lower.ends_with(".csv") {
                csvs.push(inside);
            }
        }
        if !migrations.is_empty() {
            // A pasta das migrações é a parte comum a elas (ex.: flyway/sql).
            let folders: Vec<Vec<&str>> = migrations.iter().map(|m| m.rsplit_once('/').map_or("", |(d, _)| d).split('/').collect()).collect();
            let mut common: Vec<&str> = folders[0].clone();
            for folder in &folders[1..] {
                let shared = common.iter().zip(folder).take_while(|(a, b)| a == b).count();
                common.truncate(shared);
            }
            return Ok(Format::Flyway { pasta: common.join("/") });
        }
        csvs.sort();
        if csvs.is_empty() {
            return Err("Não achei migrações SQL do Flyway nem arquivos CSV nesse link".into());
        }
        if csvs.len() > 40 {
            return Err(format!("Esse link tem {} CSVs; aponte para uma pasta com até 40", csvs.len()));
        }
        Ok(Format::Csv { arquivos: csvs })
    }

    /// Identificador estável da base sugerida.
    pub fn id_for(link: &Link) -> String {
        format!("gh-{}-{}", super::csv::name(&link.owner).to_lowercase(), super::csv::name(&link.repo).to_lowercase()).replace('_', "-")
    }

    pub fn base(link: &Link, name: String, description: String, license: String, commit: String, bytes: u64, sha256: String, format: Format) -> Base {
        let keywords = super::fold(&name).split_whitespace().filter(|w| w.len() >= 5).map(str::to_string).collect();
        Base {
            id: id_for(link), name, publisher: link.owner.clone(), description, license, repository: format!("{}/{}", link.owner, link.repo),
            commit, bytes, sha256, format, version_query: None, keywords, guide: String::new(), examples: Vec::new(), origin: "voce".into(),
        }
    }
}

/// Ordem do Flyway: `beforeMigrate`, migrações versionadas (`B`/`V<n>__`) pelo número e `afterMigrate`.
fn plan(scripts: &BTreeMap<String, String>) -> Result<Vec<(&str, &str)>, String> {
    let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_string();
    let callback = |wanted: &str| scripts.iter().find(|(p, _)| name(p) == wanted).map(|(p, s)| (p.as_str(), s.as_str()));
    let mut versioned: Vec<(u64, bool, &str, &str)> = scripts.iter().filter_map(|(path, sql)| {
        let file = name(path);
        let kind = file.chars().next()?;
        let (number, _) = file.get(1..)?.split_once("__")?;
        let number = number.replace('_', "").parse().ok()?;
        matches!(kind, 'B' | 'V').then_some((number, kind == 'V', path.as_str(), sql.as_str()))
    }).collect();
    if versioned.is_empty() {
        return Err("A fonte não tem migrações reconhecidas".into());
    }
    versioned.sort();
    let mut out: Vec<(&str, &str)> = callback("beforeMigrate.sql").into_iter().collect();
    out.extend(versioned.into_iter().map(|(_, _, p, s)| (p, s)));
    out.extend(callback("afterMigrate.sql"));
    Ok(out)
}

/// Roda um script inteiro, aceitando comandos que devolvem linhas (verificações e PRAGMAs).
fn run_script(db: &Connection, sql: &str) -> rusqlite::Result<()> {
    use rusqlite::fallible_iterator::FallibleIterator;
    let mut batch = rusqlite::Batch::new(db, sql);
    while let Some(mut statement) = batch.next()? {
        let mut rows = statement.raw_query();
        while rows.next()?.is_some() {}
    }
    Ok(())
}

/// Monta o banco a partir do pacote conferido. Como no Flyway, cada migração roda numa
/// transação (o `PRAGMA defer_foreign_keys` só vale assim); scripts que ligam ou desligam as
/// chaves estrangeiras rodam fora dela. No fim, chaves e integridade são conferidas e a
/// tabela de busca sem acentos é criada. Devolve a versão dos dados.
pub fn build(entry: &Base, archive: &Path, out: &Path) -> Result<Option<String>, String> {
    let _ = fs::remove_file(out);
    let db = Connection::open(out).map_err(|_| "Não foi possível criar a base")?;
    match &entry.format {
        Format::Flyway { pasta } => migrate(&db, &scripts(archive, pasta)?)?,
        Format::Csv { arquivos } => csv::import(&db, &files(archive, arquivos)?)?,
    }
    let broken: i64 = db.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).map_err(|_| "Falha ao conferir a base")?;
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|_| "Falha ao conferir a base")?;
    if broken > 0 || integrity != "ok" {
        return Err("A base montada não passou na conferência".into());
    }
    db.execute_batch("BEGIN").and_then(|()| search_columns(&db, matches!(entry.format, Format::Csv { .. }))).and_then(|()| db.execute_batch("COMMIT")).map_err(|_| "Falha ao preparar a busca da base")?;
    let version = entry.version_query.as_deref().and_then(|q| db.query_row(q, [], |r| r.get::<_, String>(0)).ok());
    db.execute_batch("PRAGMA journal_mode = DELETE; VACUUM;").map_err(|_| "Falha ao compactar a base")?;
    Ok(version)
}

fn migrate(db: &Connection, scripts: &BTreeMap<String, String>) -> Result<(), String> {
    let steps = plan(scripts)?;
    for (path, sql) in steps {
        let toggles_keys = sql.to_ascii_uppercase().replace("DEFER_FOREIGN_KEYS", "").contains("FOREIGN_KEYS");
        let result = if toggles_keys {
            run_script(&db, sql)
        } else {
            db.execute_batch("BEGIN").and_then(|()| run_script(&db, sql)).and_then(|()| db.execute_batch("COMMIT"))
        };
        if let Err(error) = result {
            let _ = db.execute_batch("ROLLBACK");
            return Err(format!("A montagem da base parou em {path}: {error}"));
        }
    }
    Ok(())
}

fn tables(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'flyway_%' ORDER BY name")?;
    let names = statement.query_map([], |r| r.get(0))?.collect();
    names
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Colunas `<coluna>_BUSCA`: descrições, nomes e textos em minúsculas e sem acentos, para o
/// modelo local achar itens com `LIKE '%termo%'`.
fn search_columns(db: &Connection, all_text: bool) -> rusqlite::Result<()> {
    for table in tables(db)? {
        let columns: Vec<String> = {
            let mut statement = db.prepare(&format!("SELECT name, type FROM pragma_table_info({})", quote_literal(&table)))?;
            let rows: Vec<(String, String)> = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            rows.into_iter().filter(|(name, kind)| kind.eq_ignore_ascii_case("TEXT") && !name.ends_with("_BUSCA")
                && (all_text || name.contains("DESCRICAO") || name.contains("NOME") || name.contains("TEXTO"))).map(|(n, _)| n).collect()
        };
        for column in columns {
            // Nas bases CSV, só colunas com letras (códigos e números não precisam de busca).
            let has_letters = db.query_row(&format!("SELECT 1 FROM {} WHERE {} GLOB '*[A-Za-z]*' LIMIT 1", quote(&table), quote(&column)), [], |_| Ok(())).is_ok();
            if all_text && !has_letters {
                continue;
            }
            let search = format!("{column}_BUSCA");
            db.execute_batch(&format!("ALTER TABLE {} ADD COLUMN {} TEXT", quote(&table), quote(&search)))?;
            let values: Vec<(i64, String)> = {
                let mut statement = db.prepare(&format!("SELECT rowid, {} FROM {} WHERE {0} IS NOT NULL", quote(&column), quote(&table)))?;
                let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                rows
            };
            let mut update = db.prepare(&format!("UPDATE {} SET {} = ?1 WHERE rowid = ?2", quote(&table), quote(&search)))?;
            for (row, text) in values {
                update.execute(rusqlite::params![fold(&text).trim(), row])?;
            }
        }
    }
    Ok(())
}

fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Esquema compacto para o modelo: tabelas com linhas, colunas e contagem.
pub fn schema(db: &Path) -> Result<String, String> {
    let db = open(db)?;
    let mut out = String::new();
    for table in tables(&db).map_err(|_| "Falha ao ler a base")? {
        let count: i64 = db.query_row(&format!("SELECT count(*) FROM {}", quote(&table)), [], |r| r.get(0)).unwrap_or(0);
        if count == 0 {
            continue;
        }
        let mut statement = db.prepare(&format!("SELECT name FROM pragma_table_info({})", quote_literal(&table))).map_err(|_| "Falha ao ler a base")?;
        let columns: Vec<String> = statement.query_map([], |r| r.get(0)).map_err(|_| "Falha ao ler a base")?.filter_map(Result::ok).collect();
        out.push_str(&format!("{table}({}) · {count} linhas\n", columns.join(", ")));
    }
    Ok(out)
}

fn open(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|_| "Base indisponível; instale-a de novo em Configurações")?;
    db.execute_batch("PRAGMA query_only = ON").map_err(|_| "Base indisponível")?;
    Ok(db)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Havia mais linhas além do limite.
    pub more: bool,
}

pub const MAX_ROWS: usize = 30;
const MAX_CELL: usize = 400;
const TIME_LIMIT: Duration = Duration::from_secs(3);

/// Termos comparados às colunas `_BUSCA` ficam como elas: minúsculos, sem acentos, e com
/// espaços virando `%` (as palavras podem estar separadas no texto original).
pub fn normalize_search(sql: &str) -> String {
    let upper = sql.to_ascii_uppercase();
    let mut out = String::with_capacity(sql.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(found) = upper[from..].find("_BUSCA") {
        let mut at = from + found + "_BUSCA".len();
        from = at;
        let rest = &upper[at..];
        let skip = rest.len() - rest.trim_start().len();
        let rest = rest.trim_start();
        let keyword = ["NOT LIKE", "LIKE", "="].into_iter().find(|k| rest.starts_with(k));
        let Some(keyword) = keyword else { continue };
        at += skip + keyword.len();
        let rest = &sql[at..];
        at += rest.len() - rest.trim_start().len();
        if !sql[at..].starts_with('\'') {
            continue;
        }
        let Some(end) = sql[at + 1..].find('\'').map(|e| at + 1 + e) else { break };
        let literal: String = fold(&sql[at + 1..end].replace('%', " % ")).trim().split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
        let pattern = literal.replace(" % ", "%").replace("% ", "%").replace(" %", "%").replace(' ', "%");
        let original = &sql[at + 1..end];
        let pattern = format!("{}{}{}", if original.starts_with('%') { "%" } else { "" }, pattern.trim_matches('%'), if original.ends_with('%') { "%" } else { "" });
        out.push_str(&sql[copied..at + 1]);
        out.push_str(&pattern);
        copied = end;
        from = end + 1;
    }
    out.push_str(&sql[copied..]);
    out
}

/// Executa uma única consulta somente leitura (SELECT/WITH), com limite de linhas e de tempo.
pub fn query(db: &Path, sql: &str) -> Result<Table, String> {
    let sql = sql.trim().trim_end_matches(';').trim();
    let head = sql.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    if head != "SELECT" && head != "WITH" {
        return Err("A consulta precisa ser um único SELECT".into());
    }
    let db = open(db)?;
    let interrupt = db.get_interrupt_handle();
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let timer = std::thread::spawn(move || {
        if wait.recv_timeout(TIME_LIMIT).is_err() {
            interrupt.interrupt();
        }
    });
    let result = (|| {
        let sql = normalize_search(sql);
        let mut statement = db.prepare(&sql).map_err(|e| format!("SQL inválido: {e}"))?;
        if !statement.readonly() {
            return Err("A consulta precisa ser somente leitura".to_string());
        }
        let columns: Vec<String> = statement.column_names().into_iter().map(str::to_string).collect();
        let mut rows = statement.raw_query();
        let mut out = Vec::new();
        let mut more = false;
        while let Some(row) = rows.next().map_err(|e| format!("Erro na consulta: {e}"))? {
            if out.len() == MAX_ROWS {
                more = true;
                break;
            }
            out.push((0..columns.len()).map(|i| cell(row.get_ref(i).ok())).collect());
        }
        Ok(Table { columns, rows: out, more })
    })();
    let _ = done.send(());
    let _ = timer.join();
    result
}

fn cell(value: Option<rusqlite::types::ValueRef<'_>>) -> String {
    use rusqlite::types::ValueRef;
    let text = match value {
        None | Some(ValueRef::Null) => String::new(),
        Some(ValueRef::Integer(n)) => n.to_string(),
        Some(ValueRef::Real(n)) => n.to_string(),
        Some(ValueRef::Text(t)) => String::from_utf8_lossy(t).replace("\\n", " ").replace('\n', " "),
        Some(ValueRef::Blob(_)) => "[binário]".into(),
    };
    if text.chars().count() > MAX_CELL { format!("{}…", text.chars().take(MAX_CELL).collect::<String>()) } else { text }
}

/// Texto minúsculo e sem acentos, com palavras separadas por um espaço (e espaços nas pontas).
pub(crate) fn fold(text: &str) -> String {
    let mut out = String::from(" ");
    for c in text.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ã' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'õ' | 'ô' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

/// A pergunta cita algum termo da base?
pub fn matches(entry: &Base, message: &str) -> bool {
    let text = fold(message);
    entry.keywords.iter().any(|term| text.contains(&format!(" {term} ")))
}

/// Termos tirados dos nomes das tabelas (ex.: MUNICIPIOS → "municipios" e "municipio"), para
/// bases adicionadas por link, que não têm palavras-chave curadas.
pub fn table_terms(db: &Path) -> Vec<String> {
    let Ok(db) = open(db) else { return Vec::new() };
    let generic = ["tabela", "dados", "codigo", "descricao", "lista", "geral", "versao", "base"];
    let mut out = Vec::new();
    for table in tables(&db).unwrap_or_default() {
        for word in table.to_lowercase().split('_').filter(|w| w.len() >= 5 && !generic.contains(w)) {
            out.push(word.to_string());
            if let Some(singular) = word.strip_suffix('s') { out.push(singular.to_string()); }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Pergunta que cita uma das tabelas da base.
pub fn matches_tables(terms: &[String], message: &str) -> bool {
    let text = fold(message);
    terms.iter().any(|term| text.contains(&format!(" {term} ")))
}

pub fn sql_schema() -> Value {
    json!({"type":"object","properties":{"sql":{"type":"string"}},"required":["sql"],"additionalProperties":false})
}

/// Pedido ao modelo local: um SELECT do SQLite para a pergunta, a partir do guia e do esquema.
/// Na segunda tentativa, leva a consulta anterior e o erro.
pub fn sql_request(entry: &Base, schema: &str, message: &str, previous: Option<(&str, &str)>) -> Vec<Value> {
    let system = format!(
        "Você escreve UMA consulta SQLite somente leitura (SELECT ou WITH) que busca, na base \"{}\" ({}), os dados para responder à pergunta. \
Use só as tabelas e colunas abaixo. Traga as colunas de código e descrição úteis, filtre pela vigência quando fizer sentido e termine com LIMIT {}. \
Para achar itens por nome, use as colunas terminadas em _BUSCA (minúsculas, sem acento) com LIKE '%termo%', usando a raiz da palavra no singular. \
Use exatamente os nomes de colunas do esquema, com o prefixo da tabela (ex.: ALRE_TBTO_ID, não TBTO_ID). Responda só com JSON {{\"sql\": \"...\"}}.\n\n{}\n\nExemplos:\n{}\n\nEsquema:\n{}",
        entry.name, entry.publisher, MAX_ROWS, if entry.guide.is_empty() { "Base adicionada por link: sem guia curado; use o esquema." } else { &entry.guide },
        entry.examples.iter().map(|(q, sql)| format!("Pergunta: {q}\nSQL: {sql}")).collect::<Vec<_>>().join("\n"), schema
    );
    let mut user = format!("Pergunta: {message}");
    if let Some((sql, error)) = previous {
        user.push_str(&format!("\n\nA consulta anterior falhou ou não trouxe linhas:\n{sql}\nMotivo: {error}\nEscreva outra consulta."));
    }
    vec![json!({"role":"system","content":system}), json!({"role":"user","content":user})]
}

/// Linhas do esquema das tabelas citadas no SQL, para a nova tentativa usar as colunas certas.
pub fn related(schema: &str, sql: &str) -> String {
    let upper = format!(" {} ", sql.to_ascii_uppercase().replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', " "));
    schema.lines().filter(|line| line.split('(').next().is_some_and(|table| upper.contains(&format!(" {table} ")))).collect::<Vec<_>>().join("\n")
}

pub fn parse_sql(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let sql = value["sql"].as_str()?.trim();
    (!sql.is_empty() && sql.len() <= 4000).then(|| sql.to_string())
}

/// Rótulo da fonte: nome, editor e versão dos dados (ou o commit).
pub fn source_label(entry: &Base, installed: &Installed) -> String {
    let version = installed.version.clone().unwrap_or_else(|| installed.commit.chars().take(7).collect());
    format!("{} · {} · {version}", entry.name, entry.publisher)
}

/// Bloco para o prompt local: a consulta feita e o resultado, com a instrução de citar e não inventar.
pub fn context_block(entry: &Base, installed: &Installed, sql: &str, table: &Table) -> String {
    let mut out = format!(
        "\n\nDados da base pública \"{}\", consultada neste computador (versão {}, commit {} de {}).\nConsulta feita:\n{sql}\n",
        source_label(entry, installed),
        installed.version.as_deref().unwrap_or("sem número"),
        installed.commit.chars().take(7).collect::<String>(),
        entry.repository,
    );
    if table.rows.is_empty() {
        out.push_str("Resultado: nenhuma linha. Diga que a base não trouxe esse dado e sugira reformular; não invente valores.");
        return out;
    }
    out.push_str(&format!("Resultado ({} linhas{}):\n", table.rows.len(), if table.more { ", há mais" } else { "" }));
    out.push_str(&table.columns.join(" | "));
    out.push('\n');
    for row in &table.rows {
        out.push_str(&row.join(" | "));
        out.push('\n');
    }
    out.push_str("Responda usando só estes dados, com os códigos e valores exatos, e diga que vieram dessa base e versão. Se não bastarem para responder, diga o que falta; não invente. Não é orientação fiscal oficial.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-bases-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Pacote como o do GitHub, com migrações no estilo da Receita.
    fn archive(dir: &Path, files: &[(&str, &str)]) -> PathBuf {
        let path = dir.join("fonte.tar.gz");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(fs::File::create(&path).unwrap(), flate2::Compression::fast()));
        for (name, text) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(text.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, format!("repo-abc/{name}"), text.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        path
    }

    const ENTRY: Entry = Entry {
        id: "teste", name: "Teste", publisher: "Órgão", description: "", license: "MIT", repository: "o/r", commit: "abcdef0123",
        bytes: 0, sha256: "", migrations: "flyway/sql", version_query: "SELECT VERSAO FROM VERSAO ORDER BY ID DESC LIMIT 1",
        keywords: &["ncm", "imposto seletivo"], guide: "", examples: &[("Exemplo?", "SELECT 1")],
    };

    #[test]
    fn monta_como_o_flyway_e_consulta_so_leitura() {
        let dir = temp("monta");
        let files = [
            ("flyway/sql/criacao/beforeMigrate.sql", "PRAGMA foreign_keys = ON;"),
            ("flyway/sql/criacao/B0001__base.sql", "PRAGMA foreign_keys = OFF;
CREATE TABLE VERSAO(ID INTEGER PRIMARY KEY, VERSAO TEXT);
CREATE TABLE GRUPO(ID INTEGER PRIMARY KEY, NOME TEXT);
CREATE TABLE ITEM(CD TEXT PRIMARY KEY, DESCRICAO TEXT, GRUPO_ID INTEGER REFERENCES GRUPO(ID));
INSERT INTO GRUPO VALUES (1, 'Bebidas');
INSERT INTO ITEM VALUES ('0901', 'Café torrado', 1);
INSERT INTO VERSAO VALUES (1, 'B0001');
PRAGMA foreign_keys = ON;"),
            // Insere o filho antes do pai: só passa com a checagem adiada dentro da transação.
            ("flyway/sql/manutencao/V0002__grupo.sql", "PRAGMA defer_foreign_keys = ON;
INSERT INTO ITEM VALUES ('2203', 'Cervejas de malte', 2);
INSERT INTO GRUPO VALUES (2, 'Álcool');
INSERT INTO VERSAO VALUES (2, 'V0002');"),
            ("flyway/sql/manutencao/V0010__depois.sql", "INSERT INTO VERSAO VALUES (3, 'V0010'); SELECT 'linhas são aceitas';"),
            ("flyway/sql/criacao/afterMigrate.sql", "PRAGMA foreign_key_check; ANALYZE;"),
            ("src/main/Outro.sql", "DROP TABLE ITEM;"),
        ];
        let source = archive(&dir, &files);
        let db = dir.join("teste.db");
        assert_eq!(build(&ENTRY.base(), &source, &db).unwrap().as_deref(), Some("V0010"), "V0010 depois de V0002, fora da pasta é ignorado");

        let schema = schema(&db).unwrap();
        assert!(schema.contains("ITEM(CD, DESCRICAO, GRUPO_ID, DESCRICAO_BUSCA) · 2 linhas"), "{schema}");
        assert!(schema.contains("GRUPO(ID, NOME, NOME_BUSCA)"), "{schema}");
        let found = query(&db, "SELECT CD, DESCRICAO FROM ITEM WHERE DESCRICAO_BUSCA LIKE '%cafe%';").unwrap();
        assert_eq!(found.rows, vec![vec!["0901".to_string(), "Café torrado".to_string()]], "busca sem acento");
        assert!(query(&db, "DELETE FROM ITEM").is_err());
        assert!(query(&db, "WITH x AS (SELECT 1) DELETE FROM ITEM").is_err(), "WITH que escreve");
        assert!(query(&db, "SELECT 1; DROP TABLE ITEM").is_err(), "um comando só");
        assert!(query(&db, "SELECT * FROM NADA").unwrap_err().starts_with("SQL inválido"));
        let many = query(&db, "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100) SELECT i FROM n").unwrap();
        assert_eq!((many.rows.len(), many.more), (MAX_ROWS, true));
        let started = std::time::Instant::now();
        assert!(query(&db, "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n").is_err(), "consulta sem fim é interrompida");
        assert!(started.elapsed() < Duration::from_secs(6));
        let count: i64 = Connection::open(&db).unwrap().query_row("SELECT count(*) FROM ITEM", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2, "nada foi alterado");

        let root = dir.join("dados");
        fs::create_dir_all(super::dir(&root)).unwrap();
        assert_eq!(installed(&root, "teste"), None);
        let done = finish(&root, &ENTRY.base(), &db, Some("V0010".into())).unwrap();
        assert_eq!(installed(&root, "teste"), Some(done.clone()));
        assert_eq!((done.commit.as_str(), done.version.as_deref()), ("abcdef0123", Some("V0010")));
        assert!(!db.exists() && db_path(&root, "teste").is_file());
        remove(&root, "teste").unwrap();
        assert_eq!(installed(&root, "teste"), None);
        remove(&root, "teste").unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn montagem_com_erro_ou_chave_quebrada_falha() {
        let dir = temp("erro");
        let broken = archive(&dir, &[("flyway/sql/B0001__base.sql", "CREATE TABLE A(ID INTEGER PRIMARY KEY);"), ("flyway/sql/V0002__ruim.sql", "INSERT INTO NADA VALUES (1);")]);
        let error = build(&ENTRY.base(), &broken, &dir.join("a.db")).unwrap_err();
        assert!(error.contains("V0002__ruim.sql"), "{error}");
        let orphan = archive(&dir, &[("flyway/sql/B0001__base.sql", "PRAGMA foreign_keys = OFF; CREATE TABLE P(ID INTEGER PRIMARY KEY); CREATE TABLE F(P_ID INTEGER REFERENCES P(ID)); INSERT INTO F VALUES (9);")]);
        assert!(build(&ENTRY.base(), &orphan, &dir.join("b.db")).is_err(), "chave estrangeira órfã");
        let empty = archive(&dir, &[("README.md", "nada")]);
        assert!(build(&ENTRY.base(), &empty, &dir.join("c.db")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reconhece_perguntas_e_monta_o_contexto() {
        assert!(matches(&ENTRY.base(), "Qual o NCM do café?"));
        assert!(matches(&ENTRY.base(), "alíquota do Imposto Seletivo sobre cigarro"));
        assert!(!matches(&ENTRY.base(), "receita de bolo de cenoura"));
        assert!(!matches(&ENTRY.base(), "uma ncmx qualquer"), "palavra inteira");
        assert_eq!(parse_sql(r#"{"sql":" SELECT 1 "}"#).as_deref(), Some("SELECT 1"));
        assert_eq!(normalize_search("SELECT * FROM NCM WHERE NCM_DESCRICAO_BUSCA LIKE '%Café Torrado%' OR x = 'Piauí'"),
            "SELECT * FROM NCM WHERE NCM_DESCRICAO_BUSCA LIKE '%cafe%torrado%' OR x = 'Piauí'", "só os termos das colunas _BUSCA");
        assert_eq!(normalize_search("WHERE a.NOME_BUSCA like 'informática%' AND b.NOME_BUSCA NOT LIKE '%ção'"),
            "WHERE a.NOME_BUSCA like 'informatica%' AND b.NOME_BUSCA NOT LIKE '%cao'");
        assert_eq!(normalize_search("SELECT NOME_BUSCA FROM T"), "SELECT NOME_BUSCA FROM T");
        assert_eq!(parse_sql(r#"{"sql":""}"#), None);
        let installed = Installed { commit: "abcdef0123".into(), version: Some("V0073".into()), bytes: 1, installed_at: 0 };
        let table = Table { columns: vec!["NCM_CD".into()], rows: vec![vec!["0901".into()]], more: false };
        let block = context_block(&ENTRY.base(), &installed, "SELECT 1", &table);
        assert!(block.contains("Teste · Órgão · V0073") && block.contains("NCM_CD\n0901") && block.contains("não invente"));
        assert!(context_block(&ENTRY.base(), &installed, "SELECT 1", &Table { columns: vec![], rows: vec![], more: false }).contains("nenhuma linha"));
        let request = sql_request(&ENTRY.base(), "ITEM(CD)", "ncm do café", Some(("SELECT x", "sem linhas")));
        assert!(request[1]["content"].as_str().unwrap().contains("SELECT x"));
        assert!(request[0]["content"].as_str().unwrap().contains("Pergunta: Exemplo?\nSQL: SELECT 1"));
        let schema = "NCM(NCM_CD) · 1 linhas\nTRIBUTO(TBTO_ID) · 4 linhas\nALIQUOTA_REFERENCIA(ALRE_ID) · 3 linhas";
        assert_eq!(related(schema, "select * from aliquota_referencia r join TRIBUTO t on t.x=r.y"), "TRIBUTO(TBTO_ID) · 4 linhas\nALIQUOTA_REFERENCIA(ALRE_ID) · 3 linhas");
    }

    /// Real (rede): baixa o commit fixado da Receita, confere o hash, monta e consulta.
    /// `cargo test --lib -- --ignored base_real`
    #[test]
    #[ignore]
    fn base_real_da_receita_monta_e_responde() {
        let dir = temp("real");
        let entry = super::CATALOG.iter().find(|e| e.id == "receita-rtc").unwrap().base();
        let archive = dir.join("rtc.tar.gz");
        let token = tokio_util::sync::CancellationToken::new();
        tauri::async_runtime::block_on(crate::core::runtime::download(&entry.archive_url(), entry.bytes, &entry.sha256, &archive, &token, |_| {})).unwrap();
        let started = std::time::Instant::now();
        let db = dir.join("rtc.db");
        let version = build(&entry, &archive, &db).unwrap();
        eprintln!("montada em {:?}: {:?}, {} bytes", started.elapsed(), version, fs::metadata(&db).unwrap().len());
        assert_eq!(version.as_deref(), Some("V0073"));
        let cafe = query(&db, "SELECT NCM_CD, NCM_DESCRICAO FROM NCM WHERE NCM_DESCRICAO_BUSCA LIKE '%cafe%' LIMIT 30").unwrap();
        assert!(cafe.rows.iter().any(|r| r[0] == "0901"), "{:?}", cafe.rows);
        for (question, sql) in &entry.examples {
            let table = query(&db, sql).unwrap_or_else(|e| panic!("{question}: {e}"));
            assert!(!table.rows.is_empty(), "exemplo sem linhas: {question}");
        }
        let schema = schema(&db).unwrap();
        assert!(schema.contains("CLASSIFICACAO_TRIBUTARIA(") && schema.len() < 12_000, "{}", schema.len());
        if let Ok(out) = std::env::var("AYLA_TEST_BASE_OUT") {
            fs::copy(&db, out).unwrap();
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Real (modelo local): com um llama-server em `AYLA_TEST_LLAMA_URL` e a base montada em
    /// `AYLA_TEST_BASE_DB`, o modelo escreve o SQL de perguntas reais, como na conversa.
    #[test]
    #[ignore]
    fn base_real_modelo_escreve_sql() {
        let (Ok(url), Ok(db)) = (std::env::var("AYLA_TEST_LLAMA_URL"), std::env::var("AYLA_TEST_BASE_DB")) else { return };
        let db = PathBuf::from(db);
        let entry = super::CATALOG.iter().find(|e| e.id == "receita-rtc").unwrap().base();
        let schema = schema(&db).unwrap();
        let client = reqwest::Client::new();
        let ask = |messages: Vec<Value>| {
            let body = json!({"messages":messages,"max_tokens":700,"temperature":0.2,"chat_template_kwargs":{"enable_thinking":false},
                "response_format":{"type":"json_schema","json_schema":{"name":"sql","schema":sql_schema()}}});
            let request = client.post(format!("{url}/v1/chat/completions")).json(&body);
            tauri::async_runtime::block_on(async move { request.send().await.unwrap().json::<Value>().await.unwrap() })["choices"][0]["message"]["content"].as_str().unwrap().to_string()
        };
        let questions = std::env::var("AYLA_TEST_PERGUNTAS").unwrap_or_default();
        for question in questions.split('|').filter(|q| !q.trim().is_empty()) {
            let started = std::time::Instant::now();
            let mut previous: Option<(String, String)> = None;
            for attempt in 1..=2 {
                let raw = ask(sql_request(&entry, &schema, question, previous.as_ref().map(|(a, b)| (a.as_str(), b.as_str()))));
                let sql = parse_sql(&raw).unwrap_or_default();
                let result = query(&db, &sql);
                eprintln!("\n### {question} (tentativa {attempt}, {:?})\n{sql}\n=> {:?}", started.elapsed(), result.as_ref().map(|t| t.rows.iter().take(5).collect::<Vec<_>>()));
                match result {
                    Ok(t) if !t.rows.is_empty() => break,
                    Ok(_) => previous = Some((sql, "nenhuma linha".into())),
                    Err(e) => { let columns = related(&schema, &sql); previous = Some((sql, format!("{e}\nColunas reais das tabelas citadas:\n{columns}"))); }
                }
            }
        }
    }

    #[test]
    fn csv_vira_tabelas_e_links_do_github_sao_conferidos() {
        let text = csv::decode(b"\xEF\xBB\xBFnome;munic\xc3\xadpio;obs\n\"Escola A\";Teresina;\"tem \"\"aspas\"\"\nquebra\"\nEscola B;Parna\xc3\xadba;\n");
        assert_eq!(csv::delimiter(&text), ';');
        let rows = csv::parse(&text, ';');
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["Escola A", "Teresina", "tem \"aspas\"\nquebra"]);
        assert_eq!(csv::decode(b"caf\xe9"), "café", "Latin-1");
        assert_eq!(csv::name("Município (IBGE)"), "MUNICIPIO_IBGE");
        assert_eq!(csv::name("2024"), "T_2024");

        let dir = temp("csv");
        let source = archive(&dir, &[("dados/escolas.csv", "nome;municipio\nEscola A;Teresina\nEscola B;Parnaíba\n"), ("dados/sub/ufs.csv", "sigla,nome\nPI,Piauí\n"), ("README.md", "x")]);
        let link = link::parse("https://github.com/org-x/dados.br/tree/main/dados").unwrap();
        assert_eq!((link.owner.as_str(), link.repo.as_str(), link.reference.as_deref(), link.path.as_str()), ("org-x", "dados.br", Some("main"), "dados"));
        let format = link::detect(&source, &link.path).unwrap();
        assert_eq!(format, Format::Csv { arquivos: vec!["dados/escolas.csv".into(), "dados/sub/ufs.csv".into()] });
        let base = link::base(&link, "Escolas do Piauí".into(), String::new(), "CC-BY-4.0".into(), "abc".into(), 1, "x".into(), format);
        assert_eq!(base.id, "gh-org-x-dados-br");
        let db = dir.join("csv.db");
        build(&base, &source, &db).unwrap();
        let found = query(&db, "SELECT NOME FROM ESCOLAS WHERE MUNICIPIO_BUSCA LIKE '%parnaiba%'").unwrap();
        assert_eq!(found.rows, vec![vec!["Escola B".to_string()]]);
        assert!(matches(&base, "quais escolas do piauí"), "nome da base vira termo");
        let terms = table_terms(&db);
        assert!(terms.contains(&"escolas".to_string()) && terms.contains(&"escola".to_string()), "{terms:?}");
        assert!(matches_tables(&terms, "Qual escola fica em Teresina?"));
        assert!(!matches_tables(&terms, "receita de bolo"));

        let flyway = archive(&dir, &[("db/migracoes/V1__a.sql", "CREATE TABLE A(X);"), ("db/migracoes/sub/V2__b.sql", "INSERT INTO A VALUES (1);")]);
        assert_eq!(link::detect(&flyway, "").unwrap(), Format::Flyway { pasta: "db/migracoes".into() });
        assert!(link::detect(&archive(&dir, &[("README.md", "x")]), "").is_err());
        for bad in ["https://gitlab.com/a/b", "https://github.com/a", "https://github.com/../b", "https://github.com/a/b/issues"] {
            assert!(link::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(link::parse("https://github.com/a/b.git").unwrap().repo, "b");

        let root = dir.join("dados");
        let mut list = vec![base.clone()];
        save_user_catalog(&root, &list).unwrap();
        assert!(find(&root, "gh-org-x-dados-br").is_some() && find(&root, "receita-rtc").is_some());
        list.clear();
        remove(&root, "gh-org-x-dados-br").unwrap();
        assert!(find(&root, "gh-org-x-dados-br").is_none(), "remover tira do catálogo");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Real (rede): link do GitHub com CSV → detecção, montagem e consulta.
    /// `AYLA_TEST_LINK=https://github.com/dono/repo cargo test --lib -- --ignored link_real --nocapture`
    #[test]
    #[ignore]
    fn link_real() {
        let Ok(url) = std::env::var("AYLA_TEST_LINK") else { return };
        let dir = temp("link");
        let parsed = link::parse(&url).unwrap();
        let client = reqwest::Client::builder().user_agent("Ayla/0.1 (assistente local)").build().unwrap();
        let api = format!("https://api.github.com/repos/{}/{}", parsed.owner, parsed.repo);
        let repo: Value = tauri::async_runtime::block_on(async { client.get(&api).send().await.unwrap().json().await.unwrap() });
        let reference = parsed.reference.clone().unwrap_or_else(|| repo["default_branch"].as_str().unwrap().to_string());
        let archive = dir.join("fonte.tar.gz");
        let (bytes, _) = tauri::async_runtime::block_on(link::download(&client, &format!("https://codeload.github.com/{}/{}/tar.gz/{reference}", parsed.owner, parsed.repo), &archive)).unwrap();
        let format = link::detect(&archive, &parsed.path).unwrap();
        let base = link::base(&parsed, parsed.repo.clone(), String::new(), repo["license"]["spdx_id"].as_str().unwrap_or("").into(), reference, bytes, String::new(), format.clone());
        let db = dir.join("link.db");
        build(&base, &archive, &db).unwrap();
        eprintln!("{} bytes · {:?} · licença {}\n{}", bytes, format, base.license, schema(&db).unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }
}
