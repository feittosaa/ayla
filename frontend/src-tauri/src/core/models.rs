use super::{hardware::Hardware, preferences::Preferences, store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Text,
    Reasoning,
    Vision,
    Transcription,
    /// Voz de saída, removida em 20/11/2026. Mantida só para listar e remover
    /// modelos baixados antes; nenhum pedido escolhe esta capacidade.
    Speech,
    Embedding,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// Capacidade principal, usada na importação e em registros anteriores.
    pub capability: Capability,
    /// Capacidades adicionais servidas pelo mesmo arquivo. Registros antigos
    /// não têm o campo e continuam servindo apenas `capability`.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    pub path: PathBuf,
    pub companion: Option<PathBuf>,
    pub bytes: u64,
    pub sha256: String,
    /// Entrada do catálogo que originou o modelo, quando baixado pela Ayla.
    #[serde(default)]
    pub catalog: Option<String>,
    /// Memória medida no contexto de referência, com margem; ausente em importações.
    #[serde(default)]
    pub memory_bytes: Option<u64>,
}
/// Contexto em que o consumo dos modelos do catálogo foi medido.
pub const MEASURED_CONTEXT: u32 = 8192;
const KV_BYTES_PER_TOKEN: u64 = 1024 * 256;
impl Model {
    pub fn supports(&self, kind: Capability) -> bool {
        (self.capability == kind || self.capabilities.contains(&kind))
            && (kind != Capability::Vision || self.companion.is_some())
    }
    pub fn estimated_memory(&self, context: u32) -> u64 {
        match self.memory_bytes {
            // Medição real do catálogo; só o contexto além da referência é estimado.
            Some(measured) => measured.saturating_add(
                u64::from(context.saturating_sub(MEASURED_CONTEXT)) * KV_BYTES_PER_TOKEN,
            ),
            // Estimativa conservadora de pesos + buffers; runtime pode rejeitar
            // arquiteturas/contextos mais caros. Não é uma garantia de VRAM.
            None => self
                .bytes
                .saturating_mul(2)
                .saturating_add(u64::from(context) * KV_BYTES_PER_TOKEN),
        }
    }
}

pub struct CatalogFile {
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub projector: bool,
}
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub capabilities: &'static [Capability],
    /// RAM + VRAM medidas com projetor carregado e contexto 2048–8192
    /// (benchmark de 19/11/2026), antes da margem de segurança.
    pub measured_bytes: u64,
    pub files: &'static [CatalogFile],
    /// Motor necessário: "llama" (pacote preferido) ou "whisper".
    pub runtime: &'static str,
}
impl CatalogEntry {
    pub fn download_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
    pub fn memory_bytes(&self) -> u64 {
        self.measured_bytes.saturating_mul(6) / 5
    }
    pub fn fits(&self, budget: u64, context: u32) -> bool {
        self.memory_bytes().saturating_add(
            u64::from(context.saturating_sub(MEASURED_CONTEXT)) * KV_BYTES_PER_TOKEN,
        ) <= budget
    }
}
const CONVERSATION: &[Capability] = &[Capability::Text, Capability::Reasoning, Capability::Vision];
/// Conjunto inicial aprovado: revisões fixas e verificação SHA-256.
pub fn catalog() -> &'static [CatalogEntry] {
    &[
        CatalogEntry {
            id: "gemma-4-e4b",
            name: "Gemma 4 E4B",
            summary: "Modelo principal: conversa, raciocínio e imagens em português.",
            capabilities: CONVERSATION,
            measured_bytes: 7_464_751_104,
            runtime: "llama",
            files: &[
                CatalogFile { url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/4b4a2c1d584be7264f87aac328a1bc739ce81b6c/gemma-4-E4B_q4_0-it.gguf", bytes: 5_154_941_280, sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee", projector: false },
                CatalogFile { url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/4b4a2c1d584be7264f87aac328a1bc739ce81b6c/gemma-4-E4B-it-mmproj.gguf", bytes: 991_552_256, sha256: "7498a37cb619e55f2fcf87eb931f56e99389ed6d432e4c5c66110694c0d65578", projector: true },
            ],
        },
        CatalogEntry {
            id: "qwen3.5-4b",
            name: "Qwen3.5 4B",
            summary: "Alternativa leve para computadores com menos memória.",
            capabilities: CONVERSATION,
            measured_bytes: 4_748_759_040,
            runtime: "llama",
            files: &[
                CatalogFile { url: "https://huggingface.co/ggml-org/Qwen3.5-4B-GGUF/resolve/8d43d974efc48399cb58957ad49c8b26bf5a109c/Qwen3.5-4B-Q4_K_M.gguf", bytes: 3_032_091_584, sha256: "217002fa8008ef4fc6d548353a3aebef055a96e6ff8e0e2c0953ef8254a89c14", projector: false },
                CatalogFile { url: "https://huggingface.co/ggml-org/Qwen3.5-4B-GGUF/resolve/8d43d974efc48399cb58957ad49c8b26bf5a109c/mmproj-Qwen3.5-4B-Q8_0.gguf", bytes: 366_894_560, sha256: "5a5ba634d46f9888599963bfc52194eadf099e1efdbee46d71957d911cf62654", projector: true },
            ],
        },
        CatalogEntry {
            id: "embeddinggemma-300m",
            name: "Memória da Ayla (EmbeddingGemma 300M)",
            summary: "Permite lembrar e encontrar memórias por significado. Roda na CPU, ao lado da conversa.",
            capabilities: &[Capability::Embedding],
            // RSS medido na CPU com contexto 2048 (benchmark de 19/11/2026).
            measured_bytes: 406_847_488,
            runtime: "llama",
            files: &[
                CatalogFile { url: "https://huggingface.co/ggml-org/embeddinggemma-300M-qat-q4_0-GGUF/resolve/8dd0ca2a66a8f14470acb0e2a71f801afbc5fb73/embeddinggemma-300M-qat-Q4_0.gguf", bytes: 277_852_192, sha256: "50d28e22432a148f6f8a86eab3700f92add5d1f54baf7790675a2a4dadbccf26", projector: false },
            ],
        },
        CatalogEntry {
            id: "whisper-small",
            name: "Transcrição de voz (Whisper small)",
            summary: "Entende o que você fala. 6,9% de erro em português no benchmark, ~1 s por fala curta na CPU.",
            capabilities: &[Capability::Transcription],
            // Pico de RAM do whisper-cli na CPU (benchmark FLEURS de 19/11/2026).
            measured_bytes: 787_480_576,
            runtime: "whisper",
            files: &[
                CatalogFile { url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin", bytes: 487_601_967, sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b", projector: false },
            ],
        },
        CatalogEntry {
            id: "whisper-tiny",
            name: "Prévia da transcrição (Whisper tiny)",
            summary: "Mostra o texto enquanto você fala; o texto final usa o Whisper small.",
            capabilities: &[Capability::Transcription],
            // Pico de RAM do whisper-cli na CPU (benchmark FLEURS de 19/11/2026).
            measured_bytes: 185_597_952,
            runtime: "whisper",
            files: &[
                CatalogFile { url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.bin", bytes: 77_691_713, sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21", projector: false },
            ],
        },
    ]
}
/// Melhor modelo de conversa que cabe no orçamento: o de maior consumo medido.
pub fn recommended(budget: u64, context: u32) -> Option<&'static CatalogEntry> {
    catalog()
        .iter()
        .filter(|entry| entry.capabilities.contains(&Capability::Text) && entry.fits(budget, context))
        .max_by_key(|entry| entry.measured_bytes)
}

pub fn request_kind(text: &str, image: bool) -> Capability {
    if image {
        return Capability::Vision;
    }
    let lower = text.to_lowercase();
    if text.len() > 3000
        || [
            "analise",
            "análise",
            "planeje",
            "compare",
            "raciocine",
            "resolva",
            "investigue",
            "analyze",
            "reason",
            "plan ",
        ]
        .iter()
        .any(|word| lower.contains(word))
    {
        Capability::Reasoning
    } else {
        Capability::Text
    }
}

pub fn memory_budget(hardware: &Hardware, preferences: &Preferences) -> u64 {
    let reserve = 2 * 1024 * 1024 * 1024;
    let total = hardware
        .ram_total_bytes
        .saturating_mul(u64::from(preferences.percent()))
        / 100;
    total.min(hardware.ram_available_bytes.saturating_sub(reserve))
}
pub fn choose(
    models: &[Model],
    kind: Capability,
    budget: u64,
    context: u32,
    selected: Option<&str>,
) -> Result<Model, String> {
    let mut candidates: Vec<_> = models
        .iter()
        .filter(|m| {
            m.supports(kind)
                && m.path.is_file()
                && m.estimated_memory(context) <= budget
                && selected.map(|id| id == m.id).unwrap_or(true)
        })
        .cloned()
        .collect();
    if matches!(kind, Capability::Text | Capability::Reasoning | Capability::Vision) {
        // Conversa: o maior modelo que cabe tende a responder melhor.
        candidates.sort_by_key(|m| std::cmp::Reverse(m.bytes));
    } else {
        // Voz e memória mantêm a escolha estável; vetores são separados por modelo.
        candidates.sort_by_key(|m| m.bytes);
    }
    candidates.into_iter().next().ok_or_else(||"Nenhum modelo deste tipo cabe no orçamento atual. Importe um modelo compatível ou ajuste o perfil.".into())
}

fn hash(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}

pub fn import(
    source: &Path,
    companion: Option<&Path>,
    root: &Path,
    name: String,
    capability: Capability,
) -> io::Result<Model> {
    if name.trim().is_empty() {
        return Err(io::Error::other("Dê um nome ao modelo"));
    }
    if capability == Capability::Speech {
        return Err(io::Error::other("A Ayla responde em texto; modelos de voz não são mais usados"));
    }
    let source = source.canonicalize()?;
    let directory = root.join(store::id());
    fs::create_dir_all(&directory)?;
    let result = (|| {
        let mut file = fs::File::open(&source)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if capability != Capability::Transcription && magic != *b"GGUF" {
            return Err(io::Error::other(
                "Este tipo exige um arquivo GGUF compatível com llama.cpp",
            ));
        }
        if capability == Capability::Transcription && magic != *b"lmgg" && magic != *b"ggml" {
            return Err(io::Error::other(
                "Transcrição exige um modelo GGML compatível com whisper.cpp",
            ));
        }
        let path = directory.join(if capability == Capability::Transcription {
            "model.bin"
        } else {
            "model.gguf"
        });
        let bytes = fs::copy(&source, &path)?;
        let mut total = bytes;
        let companion = match companion {
            Some(file) => {
                let dest = directory.join("projector.gguf");
                total += fs::copy(file, &dest)?;
                Some(dest)
            }
            None => None,
        };
        if capability == Capability::Vision && companion.is_none() {
            return Err(io::Error::other(
                "Visão exige também o projetor multimodal compatível",
            ));
        }
        let checksum = hash(&path)?;
        Ok(Model {
            id: directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            name,
            capability,
            capabilities: Vec::new(),
            path,
            companion,
            bytes: total,
            sha256: checksum,
            catalog: None,
            memory_bytes: None,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_reserves_system_memory_and_handles_unknown_hardware() {
        let hw = Hardware {
            ram_total_bytes: 16 << 30,
            ram_available_bytes: 6 << 30,
            logical_cpus: 4,
            gpus: vec![],
        };
        assert_eq!(memory_budget(&hw, &Preferences::initial()), 4 << 30);
        let empty = Hardware {
            ram_total_bytes: 0,
            ram_available_bytes: 0,
            logical_cpus: 1,
            gpus: vec![],
        };
        assert_eq!(memory_budget(&empty, &Preferences::initial()), 0);
    }
    fn model(id: &str, file: &Path, bytes: u64, capabilities: Vec<Capability>, projector: bool) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            capability: Capability::Text,
            capabilities,
            path: file.into(),
            companion: projector.then(|| file.into()),
            bytes,
            sha256: String::new(),
            catalog: None,
            memory_bytes: None,
        }
    }
    #[test]
    fn legacy_records_keep_their_single_capability() {
        let legacy: Model = serde_json::from_str(
            r#"{"id":"a","name":"a","capability":"reasoning","path":"/x","companion":null,"bytes":1,"sha256":""}"#,
        )
        .unwrap();
        assert!(legacy.supports(Capability::Reasoning));
        assert!(!legacy.supports(Capability::Text));
        assert!(legacy.catalog.is_none() && legacy.memory_bytes.is_none());
    }
    #[test]
    fn conversation_prefers_largest_fitting_model_and_vision_requires_projector() {
        let root = std::env::temp_dir().join(format!("ayla-choose-{}", store::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("m.gguf");
        fs::write(&file, b"GGUF").unwrap();
        let all = vec![Capability::Text, Capability::Reasoning, Capability::Vision];
        let models = vec![
            model("small", &file, 1 << 30, all.clone(), false),
            model("large", &file, 3 << 30, all.clone(), true),
            model("huge", &file, 20 << 30, all, true),
        ];
        let budget = 8 << 30;
        assert_eq!(choose(&models, Capability::Text, budget, 2048, None).unwrap().id, "large");
        assert_eq!(choose(&models, Capability::Reasoning, budget, 2048, None).unwrap().id, "large");
        let only_small = &models[..1];
        assert!(choose(only_small, Capability::Vision, budget, 2048, None).is_err());
        assert_eq!(choose(&models, Capability::Text, budget, 2048, Some("small")).unwrap().id, "small");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn measured_memory_replaces_generic_estimate() {
        let mut m = model("m", Path::new("/x"), 5 << 30, vec![], false);
        assert_eq!(m.estimated_memory(2048), (10 << 30) + 2048 * KV_BYTES_PER_TOKEN);
        m.memory_bytes = Some(7 << 30);
        assert_eq!(m.estimated_memory(8192), 7 << 30);
        assert_eq!(m.estimated_memory(9192), (7 << 30) + 1000 * KV_BYTES_PER_TOKEN);
    }
    #[test]
    fn catalog_is_pinned_verified_and_recommends_by_budget() {
        for entry in catalog() {
            assert!(entry.files.iter().filter(|f| f.projector).count() <= 1);
            for file in entry.files {
                assert!(file.url.starts_with("https://huggingface.co/"));
                let revision = file.url.split("/resolve/").nth(1).unwrap().split('/').next().unwrap();
                assert!(revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(file.sha256.len() == 64 && file.sha256.chars().all(|c| c.is_ascii_hexdigit()));
                assert!(file.bytes > 0);
            }
        }
        for entry in catalog() {
            assert!(matches!(entry.runtime, "llama" | "whisper"));
            assert!(!entry.capabilities.contains(&Capability::Speech), "a Ayla não baixa voz de saída");
        }
        assert_eq!(recommended(15 << 30, 8192).unwrap().id, "gemma-4-e4b");
        assert_eq!(recommended(7 << 30, 8192).unwrap().id, "qwen3.5-4b");
        assert!(recommended(4 << 30, 8192).is_none());
    }
    #[test]
    fn invalid_import_does_not_touch_source_or_leave_partial_assets() {
        let root = std::env::temp_dir().join(format!("ayla-model-test-{}", store::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("invalid.gguf");
        fs::write(&source, b"invalid").unwrap();
        assert!(import(
            &source,
            None,
            &root.join("models"),
            "teste".into(),
            Capability::Text
        )
        .is_err());
        assert_eq!(fs::read(&source).unwrap(), b"invalid");
        assert_eq!(fs::read_dir(root.join("models")).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
