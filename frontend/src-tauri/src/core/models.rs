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
    Speech,
    Embedding,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub capability: Capability,
    pub path: PathBuf,
    pub companion: Option<PathBuf>,
    pub bytes: u64,
    pub sha256: String,
}
impl Model {
    pub fn estimated_memory(&self, context: u32) -> u64 {
        // Estimativa conservadora de pesos + buffers; runtime pode rejeitar
        // arquiteturas/contextos mais caros. Não é uma garantia de VRAM.
        self.bytes
            .saturating_mul(2)
            .saturating_add(u64::from(context) * 1024 * 256)
    }
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
            m.capability == kind
                && m.path.is_file()
                && m.estimated_memory(context) <= budget
                && selected.map(|id| id == m.id).unwrap_or(true)
        })
        .cloned()
        .collect();
    candidates.sort_by_key(|m| m.bytes);
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
fn copy_assets(source: &Path, target: &Path) -> io::Result<u64> {
    fs::create_dir_all(target)?;
    let mut size = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let info = entry.file_type()?;
        if info.is_symlink() {
            return Err(io::Error::other(
                "Modelos não podem incluir atalhos de arquivos",
            ));
        }
        let destination = target.join(entry.file_name());
        size += if info.is_dir() {
            copy_assets(&entry.path(), &destination)?
        } else {
            fs::copy(entry.path(), destination)?
        };
    }
    Ok(size)
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
    let source = source.canonicalize()?;
    let directory = root.join(store::id());
    fs::create_dir_all(&directory)?;
    let result = (|| {
        let path;
        let bytes;
        if capability == Capability::Speech {
            if !source.is_dir() {
                return Err(io::Error::other(
                    "Escolha a pasta do modelo de voz, incluindo tokens e dados auxiliares",
                ));
            }
            bytes = copy_assets(&source, &directory)?;
            let files: Vec<_> = fs::read_dir(&directory)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "onnx"))
                .collect();
            if files.len() != 1 || !directory.join("tokens.txt").is_file() {
                return Err(io::Error::other("Voz: a pasta deve conter um único modelo ONNX e tokens.txt; modelos VITS suportados pelo sherpa-onnx"));
            }
            path = files[0].clone();
        } else {
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
            path = directory.join(if capability == Capability::Transcription {
                "model.bin"
            } else {
                "model.gguf"
            });
            bytes = fs::copy(&source, &path)?;
        }
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
            path,
            companion,
            bytes: total,
            sha256: checksum,
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
