//! Hand · macros: sequências de teclado e mouse montadas passo a passo ou gravadas, rodadas por um
//! botão ou por um atalho do Hyprland (`ayla --macro <id>`). A gravação lê o teclado e o mouse
//! direto do sistema (`/dev/input`, só com a pessoa no grupo `input`) enquanto o botão Gravar
//! está ligado; as teclas gravadas voltam pelos códigos físicos (`ydotool`), respeitando o layout.
use super::hand_input::{self, Botao};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum PassoMacro {
    /// Texto digitado pelo teclado virtual (`wtype`).
    Digitar { texto: String },
    /// Combinação ("ctrl+shift+t").
    Teclas { atalho: String },
    /// Teclas gravadas: códigos físicos (código, apertada) e um rótulo legível.
    Gravado { rotulo: String, teclas: Vec<(u16, bool)> },
    Clicar { botao: Botao, x: i64, y: i64, vezes: u8 },
    Arrastar { botao: Botao, de: [i64; 2], para: [i64; 2] },
    /// Positivo desce.
    Rolar { dentes: i32 },
    Esperar { ms: u64 },
    Abrir { programa: String, nome: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Macro {
    pub id: String,
    pub nome: String,
    pub passos: Vec<PassoMacro>,
}

pub fn arquivo(dir: &Path) -> PathBuf { dir.join("macros.json") }

pub fn carregar(dir: &Path) -> Vec<Macro> {
    fs::read(arquivo(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn guardar(dir: &Path, macros: &[Macro]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar as macros")?;
    let tmp = arquivo(dir).with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(macros).unwrap_or_default()).map_err(|_| "Não foi possível guardar as macros")?;
    fs::rename(&tmp, arquivo(dir)).map_err(|_| "Não foi possível guardar as macros".into())
}

pub fn validar(m: &Macro) -> Result<(), String> {
    if m.nome.trim().is_empty() { return Err("Dê um nome à macro".into()); }
    if m.passos.is_empty() { return Err("A macro não tem passos".into()); }
    if m.passos.len() > 2000 { return Err("Macro grande demais".into()); }
    if !m.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || m.id.is_empty() { return Err("Macro inválida".into()); }
    for p in &m.passos {
        match p {
            PassoMacro::Teclas { atalho } => { hand_input::combo_args(atalho)?; }
            PassoMacro::Esperar { ms } if *ms > 600_000 => return Err("Espera de mais de 10 minutos".into()),
            PassoMacro::Gravado { teclas, .. } if teclas.iter().any(|(c, _)| *c == 0 || *c > 0x2ff) => return Err("Tecla gravada inválida".into()),
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Rodar

/// Roda os passos, parando se o `token` for cancelado (botão Parar ou o atalho de parar).
pub fn rodar(dir: &Path, passos: &[PassoMacro], token: &CancellationToken) -> Result<(), String> {
    for passo in passos {
        if token.is_cancelled() { return Err("Parada".into()); }
        match passo {
            PassoMacro::Digitar { texto } => hand_input::digitar(texto)?,
            PassoMacro::Teclas { atalho } => hand_input::teclas(atalho)?,
            PassoMacro::Gravado { teclas, .. } => hand_input::teclas_brutas(dir, teclas)?,
            PassoMacro::Clicar { botao, x, y, vezes } => hand_input::clicar(dir, *botao, Some((*x, *y)), *vezes)?,
            PassoMacro::Arrastar { botao, de, para } => hand_input::arrastar(dir, *botao, (de[0], de[1]), (para[0], para[1]))?,
            PassoMacro::Rolar { dentes } => hand_input::rolar(dir, *dentes)?,
            PassoMacro::Esperar { ms } => {
                let until = Instant::now() + Duration::from_millis(*ms);
                while Instant::now() < until {
                    if token.is_cancelled() { return Err("Parada".into()); }
                    std::thread::sleep(Duration::from_millis(40).min(until.saturating_duration_since(Instant::now())));
                }
            }
            PassoMacro::Abrir { programa, .. } => super::hand::launch(programa)?,
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Gravar

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Evento {
    Tecla { codigo: u16, apertada: bool },
    Botao { botao: Botao, apertado: bool, x: i64, y: i64 },
    Roda { dentes: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bruto { pub ms: u64, pub evento: Evento }

const MODS: &[u16] = &[29, 97, 56, 100, 125, 126, 42, 54];
const SHIFT: &[u16] = &[42, 54];

/// Nome de uma tecla (código do Linux) para o rótulo; `shift` muda letras e símbolos (layout US).
pub fn nome_tecla(code: u16, shift: bool) -> String {
    const ROW1: &str = "1234567890-=";
    const ROW1S: &str = "!@#$%^&*()_+";
    let pick = |plain: char, shifted: char| (if shift { shifted } else { plain }).to_string();
    match code {
        2..=13 => pick(ROW1.chars().nth(usize::from(code - 2)).unwrap_or('?'), ROW1S.chars().nth(usize::from(code - 2)).unwrap_or('?')),
        16..=25 => { let c = "qwertyuiop".chars().nth(usize::from(code - 16)).unwrap_or('?'); if shift { c.to_ascii_uppercase().to_string() } else { c.to_string() } }
        30..=38 => { let c = "asdfghjkl".chars().nth(usize::from(code - 30)).unwrap_or('?'); if shift { c.to_ascii_uppercase().to_string() } else { c.to_string() } }
        44..=50 => { let c = "zxcvbnm".chars().nth(usize::from(code - 44)).unwrap_or('?'); if shift { c.to_ascii_uppercase().to_string() } else { c.to_string() } }
        26 => pick('[', '{'), 27 => pick(']', '}'), 39 => pick(';', ':'), 40 => pick('\'', '"'), 41 => pick('`', '~'), 43 => pick('\\', '|'),
        51 => pick(',', '<'), 52 => pick('.', '>'), 53 => pick('/', '?'), 57 => " ".into(),
        1 => "Esc".into(), 14 => "⌫".into(), 15 => "Tab".into(), 28 | 96 => "Enter".into(),
        29 | 97 => "Ctrl".into(), 42 | 54 => "Shift".into(), 56 => "Alt".into(), 100 => "AltGr".into(), 125 | 126 => "Super".into(),
        58 => "CapsLock".into(), 59..=68 => format!("F{}", code - 58), 87 => "F11".into(), 88 => "F12".into(),
        102 => "Home".into(), 103 => "↑".into(), 104 => "PgUp".into(), 105 => "←".into(), 106 => "→".into(), 107 => "End".into(),
        108 => "↓".into(), 109 => "PgDn".into(), 110 => "Insert".into(), 111 => "Delete".into(), 99 => "Print".into(),
        _ => format!("tecla {code}"),
    }
}

fn texto_simples(code: u16) -> bool { matches!(code, 2..=13 | 16..=27 | 30..=41 | 43..=53 | 57) }

/// Transforma os eventos gravados em passos: teclas em blocos (texto ou combinação), cliques,
/// cliques duplos, arrastes, rolagem e as pausas entre eles. `ultimo_clique` tira o clique final
/// (o do botão Parar na Ayla).
pub fn converter(eventos: &[Bruto], tirar_ultimo_clique: bool) -> Vec<PassoMacro> {
    let mut passos: Vec<PassoMacro> = Vec::new();
    let mut fim_anterior: Option<u64> = None;
    let mut pressed: HashSet<u16> = HashSet::new();
    let mut botoes: Vec<(Botao, i64, i64, u64)> = Vec::new();
    // Bloco de teclas em andamento: (teclas, rótulo, é combinação, último ms).
    let mut bloco: Option<(Vec<(u16, bool)>, String, bool, u64)> = None;
    let mut ultimo_clique: Option<(u64, i64, i64, Botao)> = None;

    fn pausa(passos: &mut Vec<PassoMacro>, fim: &mut Option<u64>, agora: u64) {
        if let Some(f) = *fim {
            let gap = agora.saturating_sub(f);
            if gap >= 300 { passos.push(PassoMacro::Esperar { ms: (gap / 100 * 100).min(30_000) }); }
        }
    }
    fn fechar(passos: &mut Vec<PassoMacro>, bloco: &mut Option<(Vec<(u16, bool)>, String, bool, u64)>, fim: &mut Option<u64>) {
        if let Some((teclas, rotulo, _, ultimo)) = bloco.take() {
            if !teclas.is_empty() { passos.push(PassoMacro::Gravado { rotulo: rotulo.trim().to_string(), teclas }); *fim = Some(ultimo); }
        }
    }

    for b in eventos {
        match b.evento {
            Evento::Tecla { codigo, apertada } => {
                if !apertada && !pressed.remove(&codigo) { continue; }
                if apertada && !pressed.insert(codigo) { continue; }
                let segurando_mod = pressed.iter().any(|c| MODS.contains(c) && !SHIFT.contains(c) && *c != codigo);
                let combo = apertada && !MODS.contains(&codigo) && segurando_mod;
                let novo_bloco = match &bloco {
                    None => true,
                    Some((_, _, era_combo, ultimo)) => apertada && pressed.len() == 1 && (b.ms.saturating_sub(*ultimo) > 700 || (*era_combo != combo && !MODS.contains(&codigo))),
                };
                if novo_bloco && apertada {
                    fechar(&mut passos, &mut bloco, &mut fim_anterior);
                    pausa(&mut passos, &mut fim_anterior, b.ms);
                    bloco = Some((Vec::new(), String::new(), combo, b.ms));
                }
                let Some((teclas, rotulo, era_combo, ultimo)) = bloco.as_mut() else { continue };
                teclas.push((codigo, apertada));
                *ultimo = b.ms;
                if apertada {
                    let shift = pressed.iter().any(|c| SHIFT.contains(c));
                    if combo {
                        *era_combo = true;
                        let mods: Vec<String> = [125u16, 29, 56, 100, 42].iter().filter(|m| pressed.contains(m) || pressed.contains(&match **m { 125 => 126, 29 => 97, 42 => 54, x => x })).map(|m| nome_tecla(*m, false)).collect();
                        if !rotulo.is_empty() { rotulo.push(' '); }
                        rotulo.push_str(&format!("{}+{}", mods.join("+"), nome_tecla(codigo, false).to_uppercase()));
                    } else if texto_simples(codigo) {
                        rotulo.push_str(&nome_tecla(codigo, shift));
                    } else if !MODS.contains(&codigo) {
                        rotulo.push_str(&format!(" [{}] ", nome_tecla(codigo, false)));
                    }
                }
            }
            Evento::Botao { botao, apertado: true, x, y } => {
                fechar(&mut passos, &mut bloco, &mut fim_anterior);
                botoes.push((botao, x, y, b.ms));
            }
            Evento::Botao { botao, apertado: false, x, y } => {
                let Some(pos) = botoes.iter().position(|(bt, ..)| *bt == botao) else { continue };
                let (_, x0, y0, apertou) = botoes.remove(pos);
                let parado = (x - x0).abs() <= 4 && (y - y0).abs() <= 4;
                // Clique duplo: o mesmo botão no mesmo lugar logo depois.
                if parado {
                    if let (Some((t, lx, ly, lb)), Some(PassoMacro::Clicar { vezes, .. })) = (ultimo_clique, passos.last_mut()) {
                        if lb == botao && (lx - x0).abs() <= 4 && (ly - y0).abs() <= 4 && b.ms.saturating_sub(t) < 450 && *vezes < 3 {
                            *vezes += 1;
                            ultimo_clique = Some((b.ms, x0, y0, botao));
                            fim_anterior = Some(b.ms);
                            continue;
                        }
                    }
                }
                pausa(&mut passos, &mut fim_anterior, apertou);
                passos.push(if parado { PassoMacro::Clicar { botao, x: x0, y: y0, vezes: 1 } } else { PassoMacro::Arrastar { botao, de: [x0, y0], para: [x, y] } });
                ultimo_clique = parado.then_some((b.ms, x0, y0, botao));
                fim_anterior = Some(b.ms);
            }
            Evento::Roda { dentes } => {
                fechar(&mut passos, &mut bloco, &mut fim_anterior);
                if let (Some(PassoMacro::Rolar { dentes: d }), Some(f)) = (passos.last_mut(), fim_anterior) {
                    if b.ms.saturating_sub(f) < 400 { *d += dentes; fim_anterior = Some(b.ms); continue; }
                }
                pausa(&mut passos, &mut fim_anterior, b.ms);
                passos.push(PassoMacro::Rolar { dentes });
                fim_anterior = Some(b.ms);
            }
        }
    }
    fechar(&mut passos, &mut bloco, &mut fim_anterior);
    if tirar_ultimo_clique && matches!(passos.last(), Some(PassoMacro::Clicar { .. })) { passos.pop(); }
    while matches!(passos.last(), Some(PassoMacro::Esperar { .. })) { passos.pop(); }
    while matches!(passos.first(), Some(PassoMacro::Esperar { .. })) { passos.remove(0); }
    passos
}

/// Dispositivos de teclado e mouse em `/proc/bus/input/devices` (fora os virtuais da Ayla).
pub fn dispositivos(texto: &str) -> Vec<String> {
    let mut out = Vec::new();
    for block in texto.split("\n\n") {
        let name = block.lines().find_map(|l| l.strip_prefix("N: Name=")).unwrap_or("").trim_matches('"').to_lowercase();
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers=")).unwrap_or("");
        let ev = block.lines().find_map(|l| l.strip_prefix("B: EV=")).and_then(|v| u64::from_str_radix(v.trim(), 16).ok()).unwrap_or(0);
        if name.contains("ydotool") || name.contains("virtual") || name.contains("wtype") { continue; }
        let teclado = handlers.split_whitespace().any(|h| h == "kbd") && ev & (1 << 20) != 0;
        let mouse = handlers.split_whitespace().any(|h| h.starts_with("mouse"));
        if !(teclado || mouse) { continue; }
        if let Some(event) = handlers.split_whitespace().find(|h| h.starts_with("event")) { out.push(format!("/dev/input/{event}")); }
    }
    out
}

pub struct Gravacao {
    parar: Arc<AtomicBool>,
    eventos: Arc<Mutex<Vec<Bruto>>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

static GRAVANDO: Mutex<Option<Gravacao>> = Mutex::new(None);

/// O que falta para gravar (texto para a pessoa), ou `None` se dá para gravar.
pub fn falta_para_gravar() -> Option<String> {
    let devices = dispositivos(&fs::read_to_string("/proc/bus/input/devices").unwrap_or_default());
    if devices.is_empty() { return Some("Não achei teclado nem mouse em /dev/input.".into()); }
    if devices.iter().any(|d| fs::File::open(d).is_ok()) { return None; }
    Some("Para gravar, a Ayla precisa ler o teclado e o mouse do sistema. Rode `sudo usermod -aG input $USER` e entre de novo na sessão. Isso deixa os programas do seu usuário lerem o teclado; a Ayla só lê enquanto o botão Gravar está ligado.".into())
}

const O_NONBLOCK: i32 = 0o4000;

pub fn comecar_gravacao() -> Result<(), String> {
    let mut slot = GRAVANDO.lock().map_err(|_| "Gravação ocupada")?;
    if slot.is_some() { return Err("Já está gravando".into()); }
    use std::os::unix::fs::OpenOptionsExt;
    let devices = dispositivos(&fs::read_to_string("/proc/bus/input/devices").unwrap_or_default());
    let files: Vec<fs::File> = devices.iter().filter_map(|d| fs::OpenOptions::new().read(true).custom_flags(O_NONBLOCK).open(d).ok()).collect();
    if files.is_empty() { return Err(falta_para_gravar().unwrap_or_else(|| "Não foi possível ler o teclado e o mouse".into())); }
    let parar = Arc::new(AtomicBool::new(false));
    let eventos = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();
    let threads = files.into_iter().map(|mut file| {
        let (parar, eventos) = (parar.clone(), eventos.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 24 * 64];
            while !parar.load(Ordering::Relaxed) {
                match file.read(&mut buf) {
                    Ok(n) if n >= 24 => {
                        for chunk in buf[..n - n % 24].chunks_exact(24) {
                            let kind = u16::from_ne_bytes([chunk[16], chunk[17]]);
                            let code = u16::from_ne_bytes([chunk[18], chunk[19]]);
                            let value = i32::from_ne_bytes([chunk[20], chunk[21], chunk[22], chunk[23]]);
                            let ms = start.elapsed().as_millis() as u64;
                            let evento = match (kind, code) {
                                (1, 0x110..=0x112) if value != 2 => {
                                    let (x, y) = hand_input::cursor().unwrap_or((0, 0));
                                    let botao = match code { 0x110 => Botao::Esquerdo, 0x111 => Botao::Direito, _ => Botao::Meio };
                                    Some(Evento::Botao { botao, apertado: value == 1, x, y })
                                }
                                (1, c) if c < 0x100 && value != 2 => Some(Evento::Tecla { codigo: c, apertada: value == 1 }),
                                // REL_WHEEL: positivo é para cima; nos passos, positivo desce.
                                (2, 8) => Some(Evento::Roda { dentes: -value }),
                                _ => None,
                            };
                            if let (Some(evento), Ok(mut list)) = (evento, eventos.lock()) { if list.len() < 50_000 { list.push(Bruto { ms, evento }); } }
                        }
                    }
                    _ => std::thread::sleep(Duration::from_millis(8)),
                }
            }
        })
    }).collect();
    *slot = Some(Gravacao { parar, eventos, threads });
    Ok(())
}

pub fn gravando() -> bool { GRAVANDO.lock().is_ok_and(|g| g.is_some()) }

/// Para a gravação e devolve os passos (sem o clique final, que foi no botão Parar).
pub fn parar_gravacao() -> Result<Vec<PassoMacro>, String> {
    let gravacao = GRAVANDO.lock().map_err(|_| "Gravação ocupada")?.take().ok_or("Não está gravando")?;
    gravacao.parar.store(true, Ordering::Relaxed);
    for t in gravacao.threads { let _ = t.join(); }
    let eventos = gravacao.eventos.lock().map(|e| e.clone()).unwrap_or_default();
    Ok(converter(&eventos, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(ms: u64, codigo: u16, apertada: bool) -> Bruto { Bruto { ms, evento: Evento::Tecla { codigo, apertada } } }
    fn b(ms: u64, apertado: bool, x: i64, y: i64) -> Bruto { Bruto { ms, evento: Evento::Botao { botao: Botao::Esquerdo, apertado, x, y } } }

    #[test]
    fn gravacao_vira_passos_legiveis() {
        let eventos = vec![
            // Soltar o Enter que apertou Gravar (sem ter sido apertado na gravação): ignorado.
            k(0, 28, false),
            // "oi" digitado.
            k(100, 24, true), k(150, 24, false), k(200, 23, true), k(240, 23, false),
            // Ctrl+C depois de uma pausa.
            k(1500, 29, true), k(1550, 46, true), k(1600, 46, false), k(1650, 29, false),
            // Clique duplo em (10,20).
            b(2500, true, 10, 20), b(2550, false, 10, 20), b(2700, true, 10, 20), b(2750, false, 11, 20),
            // Arraste.
            b(4000, true, 100, 100), b(4500, false, 300, 120),
            // Rolagem para baixo (roda negativa no Linux → positivo nos passos).
            Bruto { ms: 5000, evento: Evento::Roda { dentes: 1 } }, Bruto { ms: 5100, evento: Evento::Roda { dentes: 1 } },
            // Clique no botão Parar.
            b(7000, true, 50, 50), b(7050, false, 50, 50),
        ];
        let passos = converter(&eventos, true);
        assert_eq!(passos, vec![
            PassoMacro::Gravado { rotulo: "oi".into(), teclas: vec![(24, true), (24, false), (23, true), (23, false)] },
            PassoMacro::Esperar { ms: 1200 },
            PassoMacro::Gravado { rotulo: "Ctrl+C".into(), teclas: vec![(29, true), (46, true), (46, false), (29, false)] },
            PassoMacro::Esperar { ms: 800 },
            PassoMacro::Clicar { botao: Botao::Esquerdo, x: 10, y: 20, vezes: 2 },
            PassoMacro::Esperar { ms: 1200 },
            PassoMacro::Arrastar { botao: Botao::Esquerdo, de: [100, 100], para: [300, 120] },
            PassoMacro::Esperar { ms: 500 },
            PassoMacro::Rolar { dentes: 2 },
        ]);
        assert!(converter(&[], true).is_empty());
    }

    #[test]
    fn dispositivos_e_validacao() {
        let text = "I: Bus=0003\nN: Name=\"MCHOSE Ace68\"\nH: Handlers=sysrq kbd leds event4\nB: EV=120013\n\nI: Bus=0003\nN: Name=\"Wireless mouse\"\nH: Handlers=mouse0 event6\nB: EV=17\n\nN: Name=\"ydotoold virtual device\"\nH: Handlers=sysrq kbd mouse2 event20\nB: EV=120017\n\nN: Name=\"Power Button\"\nH: Handlers=kbd event1\nB: EV=3\n";
        assert_eq!(dispositivos(text), vec!["/dev/input/event4", "/dev/input/event6"]);
        let ok = Macro { id: "m-1".into(), nome: "Teste".into(), passos: vec![PassoMacro::Teclas { atalho: "ctrl+c".into() }] };
        assert!(validar(&ok).is_ok());
        assert!(validar(&Macro { nome: " ".into(), ..ok.clone() }).is_err());
        assert!(validar(&Macro { id: "../x".into(), ..ok.clone() }).is_err());
        assert!(validar(&Macro { passos: vec![PassoMacro::Esperar { ms: 700_000 }], ..ok }).is_err());
        assert_eq!(nome_tecla(30, true), "A");
        assert_eq!(nome_tecla(2, true), "!");
    }
}
