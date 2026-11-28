//! Feriados no calendário da Athena: nacionais e pontos facultativos federais (calculados, sem
//! internet, inclusive os que dependem da Páscoa), estaduais (tabela embutida, conferida pela
//! pesquisa quando há) e municipais (pesquisados uma vez por ano para a cidade da pessoa e
//! guardados neste computador).
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tipo { Nacional, Estadual, Municipal, Facultativo }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Feriado {
    pub data: String,
    pub nome: String,
    pub tipo: Tipo,
}

/// Domingo de Páscoa (algoritmo de Meeus/Jones/Butcher, calendário gregoriano).
pub fn pascoa(ano: i32) -> NaiveDate {
    let a = ano % 19;
    let b = ano / 100;
    let c = ano % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let mes = (h + l - 7 * m + 114) / 31;
    let dia = (h + l - 7 * m + 114) % 31 + 1;
    NaiveDate::from_ymd_opt(ano, mes as u32, dia as u32).expect("Páscoa")
}

fn dia(ano: i32, mes: u32, d: u32) -> Option<NaiveDate> { NaiveDate::from_ymd_opt(ano, mes, d) }
fn item(data: NaiveDate, nome: &str, tipo: Tipo) -> Feriado { Feriado { data: data.format("%Y-%m-%d").to_string(), nome: nome.into(), tipo } }

/// Feriados nacionais (Lei 662/1949, 6.802/1980 e 14.759/2023) e a Sexta-feira Santa.
pub fn nacionais(ano: i32) -> Vec<Feriado> {
    let mut out: Vec<Feriado> = [
        (1, 1, "Confraternização Universal"), (4, 21, "Tiradentes"), (5, 1, "Dia do Trabalho"), (9, 7, "Independência do Brasil"),
        (10, 12, "Nossa Senhora Aparecida"), (11, 2, "Finados"), (11, 15, "Proclamação da República"), (12, 25, "Natal"),
    ].iter().filter_map(|(m, d, n)| dia(ano, *m, *d).map(|x| item(x, n, Tipo::Nacional))).collect();
    // Consciência Negra é feriado nacional desde 2024.
    if ano >= 2024 { if let Some(x) = dia(ano, 11, 20) { out.push(item(x, "Dia Nacional de Zumbi e da Consciência Negra", Tipo::Nacional)); } }
    out.push(item(pascoa(ano) - Duration::days(2), "Sexta-feira Santa", Tipo::Nacional));
    out.sort_by(|a, b| a.data.cmp(&b.data));
    out
}

/// Pontos facultativos federais (portaria anual do governo federal).
pub fn facultativos(ano: i32) -> Vec<Feriado> {
    let p = pascoa(ano);
    let mut out = vec![
        item(p - Duration::days(48), "Carnaval", Tipo::Facultativo),
        item(p - Duration::days(47), "Carnaval", Tipo::Facultativo),
        item(p - Duration::days(46), "Quarta-feira de Cinzas (até as 14h)", Tipo::Facultativo),
        item(p + Duration::days(60), "Corpus Christi", Tipo::Facultativo),
    ];
    for (m, d, n) in [(10, 28, "Dia do Servidor Público"), (12, 24, "Véspera de Natal (após as 14h)"), (12, 31, "Véspera de Ano-Novo (após as 14h)")] {
        if let Some(x) = dia(ano, m, d) { out.push(item(x, n, Tipo::Facultativo)); }
    }
    out.sort_by(|a, b| a.data.cmp(&b.data));
    out
}

/// Unidades da federação (sigla, nome).
pub const UFS: &[(&str, &str)] = &[
    ("AC", "Acre"), ("AL", "Alagoas"), ("AP", "Amapá"), ("AM", "Amazonas"), ("BA", "Bahia"), ("CE", "Ceará"), ("DF", "Distrito Federal"),
    ("ES", "Espírito Santo"), ("GO", "Goiás"), ("MA", "Maranhão"), ("MT", "Mato Grosso"), ("MS", "Mato Grosso do Sul"), ("MG", "Minas Gerais"),
    ("PA", "Pará"), ("PB", "Paraíba"), ("PR", "Paraná"), ("PE", "Pernambuco"), ("PI", "Piauí"), ("RJ", "Rio de Janeiro"),
    ("RN", "Rio Grande do Norte"), ("RS", "Rio Grande do Sul"), ("RO", "Rondônia"), ("RR", "Roraima"), ("SC", "Santa Catarina"),
    ("SP", "São Paulo"), ("SE", "Sergipe"), ("TO", "Tocantins"),
];

pub fn nome_uf(uf: &str) -> Option<&'static str> { UFS.iter().find(|(s, _)| *s == uf).map(|(_, n)| *n) }

/// Feriados estaduais de data fixa mais conhecidos (os que coincidem com os nacionais ficam de
/// fora). A pesquisa anual, quando há nuvem, confere e completa esta lista.
const ESTADUAIS: &[(&str, u32, u32, &str)] = &[
    ("AC", 1, 23, "Dia do Evangélico"), ("AC", 6, 15, "Aniversário do Acre"), ("AC", 9, 5, "Dia da Amazônia"), ("AC", 11, 17, "Assinatura do Tratado de Petrópolis"),
    ("AL", 6, 24, "São João"), ("AL", 6, 29, "São Pedro"), ("AL", 9, 16, "Emancipação Política de Alagoas"), ("AL", 11, 30, "Dia do Evangélico"),
    ("AP", 3, 19, "Dia de São José"), ("AP", 9, 13, "Criação do Território Federal do Amapá"),
    ("AM", 9, 5, "Elevação do Amazonas à categoria de província"),
    ("BA", 7, 2, "Independência da Bahia"),
    ("CE", 3, 19, "Dia de São José"), ("CE", 3, 25, "Data Magna do Ceará"),
    ("DF", 11, 30, "Dia do Evangélico"),
    ("MA", 7, 28, "Adesão do Maranhão à Independência"),
    ("MS", 10, 11, "Criação do Estado de Mato Grosso do Sul"),
    ("PA", 8, 15, "Adesão do Pará à Independência"),
    ("PB", 8, 5, "Fundação do Estado da Paraíba"),
    ("PR", 12, 19, "Emancipação Política do Paraná"),
    ("PE", 3, 6, "Data Magna de Pernambuco"),
    ("PI", 3, 13, "Dia da Batalha do Jenipapo"), ("PI", 10, 19, "Dia do Piauí"),
    ("RJ", 4, 23, "Dia de São Jorge"),
    ("RN", 10, 3, "Mártires de Cunhaú e Uruaçu"),
    ("RS", 9, 20, "Revolução Farroupilha"),
    ("RO", 1, 4, "Criação do Estado de Rondônia"), ("RO", 6, 18, "Dia do Evangélico"),
    ("RR", 10, 5, "Criação do Estado de Roraima"),
    ("SP", 7, 9, "Revolução Constitucionalista"),
    ("SE", 7, 8, "Autonomia Política de Sergipe"),
    ("TO", 3, 18, "Autonomia do Estado do Tocantins"), ("TO", 9, 8, "Nossa Senhora da Natividade"), ("TO", 10, 5, "Criação do Estado do Tocantins"),
];

pub fn estaduais(uf: &str, ano: i32) -> Vec<Feriado> {
    ESTADUAIS.iter().filter(|(s, ..)| *s == uf).filter_map(|(_, m, d, n)| dia(ano, *m, *d).map(|x| item(x, n, Tipo::Estadual))).collect()
}

/// Os feriados da pesquisa (estaduais e municipais) guardados para a cidade e o ano.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Pesquisa {
    pub buscado_em: i64,
    pub estaduais: Vec<Feriado>,
    pub municipais: Vec<Feriado>,
}

/// Nome de arquivo para a cidade: "SP-campinas-2026.json".
pub fn chave(uf: &str, cidade: &str, ano: i32) -> String {
    let slug: String = super::hand::fold(cidade.trim()).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let slug = slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    format!("{uf}-{slug}-{ano}.json")
}

pub fn pedido(uf: &str, cidade: &str, ano: i32) -> String {
    let estado = nome_uf(uf).unwrap_or(uf);
    format!("Quais são os feriados oficiais de {ano} no estado de {estado} ({uf}) e no município de {cidade} ({uf}), Brasil? Quero só os feriados estaduais (lei estadual) e os municipais (lei municipal ou decreto da prefeitura, como aniversário da cidade e padroeiro), sem os nacionais e sem pontos facultativos. Responda só o JSON {{\"estaduais\":[{{\"data\":\"AAAA-MM-DD\",\"nome\":\"…\"}}],\"municipais\":[{{\"data\":\"AAAA-MM-DD\",\"nome\":\"…\"}}]}} com as datas de {ano}.")
}

/// Lê a resposta da pesquisa: só datas do ano, nomes curtos, sem repetir os nacionais.
pub fn ler_pesquisa(text: &str, ano: i32) -> Option<(Vec<Feriado>, Vec<Feriado>)> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let v: Value = serde_json::from_str(&text[start..=end]).ok()?;
    let nacionais: Vec<String> = nacionais(ano).into_iter().map(|f| f.data).collect();
    let lista = |key: &str, tipo: Tipo| -> Vec<Feriado> {
        let mut out: Vec<Feriado> = v[key].as_array().into_iter().flatten().filter_map(|f| {
            let data = NaiveDate::parse_from_str(f["data"].as_str()?.trim(), "%Y-%m-%d").ok().filter(|d| d.year() == ano)?;
            let nome: String = f["nome"].as_str()?.trim().chars().take(80).collect();
            (!nome.is_empty()).then(|| item(data, &nome, tipo))
        }).filter(|f| !nacionais.contains(&f.data)).take(30).collect();
        out.sort_by(|a, b| a.data.cmp(&b.data));
        out.dedup_by(|a, b| a.data == b.data && a.nome == b.nome);
        out
    };
    if v.get("estaduais").is_none() && v.get("municipais").is_none() { return None; }
    Some((lista("estaduais", Tipo::Estadual), lista("municipais", Tipo::Municipal)))
}

/// Todos os feriados entre `de` e `ate` (inclusive), para os anos que o período cobre.
/// `pesquisa(ano)`: o que foi pesquisado para a cidade naquele ano, se houver.
pub fn periodo(de: NaiveDate, ate: NaiveDate, uf: Option<&str>, pesquisa: &dyn Fn(i32) -> Option<Pesquisa>) -> Vec<Feriado> {
    let mut out = Vec::new();
    for ano in de.year()..=ate.year() {
        out.extend(nacionais(ano));
        out.extend(facultativos(ano));
        let achado = pesquisa(ano);
        if let Some(uf) = uf {
            // A pesquisa confere a tabela embutida: com ela, valem os estaduais dela.
            match &achado { Some(p) if !p.estaduais.is_empty() => out.extend(p.estaduais.clone()), _ => out.extend(estaduais(uf, ano)) }
        }
        if let Some(p) = achado { out.extend(p.municipais); }
    }
    let (de, ate) = (de.format("%Y-%m-%d").to_string(), ate.format("%Y-%m-%d").to_string());
    out.retain(|f| f.data >= de && f.data <= ate);
    // Feriado de verdade ganha do facultativo no mesmo dia.
    let oficiais: Vec<String> = out.iter().filter(|f| f.tipo != Tipo::Facultativo).map(|f| f.data.clone()).collect();
    out.retain(|f| f.tipo != Tipo::Facultativo || !oficiais.contains(&f.data));
    out.sort_by(|a, b| a.data.cmp(&b.data));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate { NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap() }

    #[test]
    fn pascoa_e_moveis() {
        assert_eq!(pascoa(2024), d("2024-03-31"));
        assert_eq!(pascoa(2025), d("2025-04-20"));
        assert_eq!(pascoa(2026), d("2026-04-05"));
        assert_eq!(pascoa(2027), d("2027-03-28"));
        let f = facultativos(2026);
        assert!(f.iter().any(|x| x.data == "2026-02-16" && x.nome == "Carnaval"));
        assert!(f.iter().any(|x| x.data == "2026-02-17"));
        assert!(f.iter().any(|x| x.data == "2026-02-18" && x.nome.starts_with("Quarta-feira de Cinzas")));
        assert!(f.iter().any(|x| x.data == "2026-06-04" && x.nome == "Corpus Christi"));
        let n = nacionais(2026);
        assert!(n.iter().any(|x| x.data == "2026-04-03" && x.nome == "Sexta-feira Santa"));
        assert!(n.iter().any(|x| x.data == "2026-11-20"), "Consciência Negra nacional desde 2024");
        assert!(!nacionais(2023).iter().any(|x| x.data == "2023-11-20"));
        assert_eq!(n.len(), 10);
    }

    #[test]
    fn estaduais_e_periodo() {
        assert!(estaduais("SP", 2026).iter().any(|f| f.data == "2026-07-09"));
        assert!(estaduais("RJ", 2026).iter().any(|f| f.data == "2026-04-23"));
        assert!(estaduais("XX", 2026).is_empty());
        let sem = |_: i32| None;
        let dez = periodo(d("2026-12-01"), d("2027-01-10"), Some("SP"), &sem);
        let datas: Vec<&str> = dez.iter().map(|f| f.data.as_str()).collect();
        assert_eq!(datas, vec!["2026-12-24", "2026-12-25", "2026-12-31", "2027-01-01"]);
        assert!(!periodo(d("2026-07-01"), d("2026-07-31"), None, &sem).iter().any(|f| f.tipo == Tipo::Estadual), "sem estado, sem estaduais");
        let pesquisa = |ano: i32| (ano == 2026).then(|| Pesquisa { buscado_em: 1, estaduais: vec![item(d("2026-07-09"), "Revolução Constitucionalista de 1932", Tipo::Estadual)], municipais: vec![item(d("2026-07-14"), "Aniversário de Campinas", Tipo::Municipal)] });
        let jul = periodo(d("2026-07-01"), d("2026-07-31"), Some("SP"), &pesquisa);
        assert_eq!(jul.iter().map(|f| (f.data.as_str(), f.tipo)).collect::<Vec<_>>(), vec![("2026-07-09", Tipo::Estadual), ("2026-07-14", Tipo::Municipal)]);
        assert_eq!(jul[0].nome, "Revolução Constitucionalista de 1932", "a pesquisa vale sobre a tabela");
    }

    #[test]
    fn resposta_da_pesquisa() {
        let text = "Aqui está:\n```json\n{\"estaduais\":[{\"data\":\"2026-07-09\",\"nome\":\"Revolução Constitucionalista\"}],\"municipais\":[{\"data\":\"2026-07-14\",\"nome\":\"Aniversário da cidade\"},{\"data\":\"2025-12-08\",\"nome\":\"Ano errado\"},{\"data\":\"2026-11-02\",\"nome\":\"Finados\"},{\"data\":\"2026-12-08\",\"nome\":\"Nossa Senhora da Conceição\"}]}\n```";
        let (e, m) = ler_pesquisa(text, 2026).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(m.iter().map(|f| f.data.as_str()).collect::<Vec<_>>(), vec!["2026-07-14", "2026-12-08"], "sem outro ano e sem nacional");
        assert!(ler_pesquisa("não sei", 2026).is_none());
        assert_eq!(chave("SP", "São José dos Campos", 2026), "SP-sao-jose-dos-campos-2026.json");
    }
}
