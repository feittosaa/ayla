use super::policy::UsageMode;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Light,
    Moderate,
    Maximum,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub onboarding_complete: bool,
    pub profile: Option<Profile>,
    pub resource_percent: Option<u8>,
    pub usage_mode: UsageMode,
    /// Plataforma de nuvem (id do catálogo; "custom" = endereço compatível próprio; vazio = nenhuma).
    pub cloud_provider: String,
    /// Endereço do provedor compatível próprio ("custom").
    pub cloud_endpoint: String,
    pub cloud_model: String,
    /// A pessoa já escolheu entre 100% local e também usar a nuvem.
    pub cloud_asked: bool,
    /// Plataforma e modelo para criar imagens (só quando a pessoa pede).
    pub image_provider: String,
    pub image_model: String,
    pub personality: String,
    pub history_messages: usize,
    pub history_chars: usize,
    pub context_tokens: u32,
    pub idle_seconds: u64,
    /// Memória longa automática; desligar pausa novas extrações sem apagar nada.
    pub memory_auto: bool,
    /// Microfone (transcrição local), ligado por padrão desde 20/11/2026 e preparado
    /// na primeira abertura. `None` vem de arquivos antigos e vale como ligado;
    /// só `Some(false)` (escolha da pessoa) desliga. A Ayla responde só em texto.
    pub voice: Option<bool>,
    /// Notificações do sistema; arquivos anteriores recebem o padrão (todas ligadas).
    pub notifications: Notifications,
    pub desktop: DesktopPreferences,
    /// Versões anteriores: uma pasta de notas escolhida. Agora vale como um cofre a mais.
    pub athena_folder: String,
    /// Cofres (pastas de notas, ex.: do Obsidian) abertos na Athena, além das notas da Ayla.
    pub athena_vaults: Vec<String>,
    /// Versionar as notas com Git (seguindo o obsidian-git do cofre), quando a pasta for um repositório.
    pub athena_git: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopPreferences {
    pub autostart: bool,
    pub background: bool,
    pub system_theme: bool,
    pub panel_color: String,
    pub panel_accent: String,
    pub panel_font: String,
    pub panel_radius: u8,
}
impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            autostart: true, background: true, system_theme: true,
            panel_color: "#202124".into(), panel_accent: "#ff9f43".into(),
            panel_font: "system-ui".into(), panel_radius: 18,
        }
    }
}
pub fn hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Notifications {
    /// Resposta pronta enquanto a janela está fora de foco.
    pub reply: bool,
    /// Preparo de modelos concluído ou com erro.
    pub setup: bool,
    /// Lembretes na hora marcada (com o texto do lembrete, mesmo com a Ayla em foco).
    pub reminders: bool,
}
impl Default for Notifications {
    fn default() -> Self {
        Self { reply: true, setup: true, reminders: true }
    }
}

/// v2: contexto padrão 2048 → 8192 (modelos com raciocínio esgotavam o limite).
const VERSION: u32 = 2;
const V1_DEFAULT_CONTEXT: u32 = 2048;

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: VERSION,
            onboarding_complete: false,
            profile: None,
            resource_percent: None,
            usage_mode: UsageMode::LocalOnly,
            cloud_provider: String::new(),
            cloud_endpoint: String::new(),
            cloud_model: String::new(),
            cloud_asked: false,
            image_provider: String::new(),
            image_model: String::new(),
            personality: String::new(),
            history_messages: 10,
            history_chars: 12000,
            context_tokens: 8192,
            idle_seconds: 120,
            memory_auto: true,
            voice: Some(true),
            notifications: Notifications::default(),
            desktop: DesktopPreferences::default(),
            athena_folder: String::new(),
            athena_vaults: Vec::new(),
            athena_git: true,
        }
    }
}
impl Preferences {
    pub fn initial() -> Self {
        Self::default()
    }
    pub fn percent(&self) -> u8 {
        self.resource_percent.unwrap_or(match self.profile {
            Some(Profile::Light) => 25,
            Some(Profile::Maximum) => 75,
            Some(Profile::Moderate) => 50,
            None => Self::automatic_percent(super::hardware::detect().ram_total_bytes),
        })
    }
    /// Plataforma efetiva: a escolhida, ou o endereço próprio de versões anteriores.
    pub fn cloud_platform(&self) -> Option<&str> {
        if !self.cloud_provider.is_empty() {
            Some(self.cloud_provider.as_str())
        } else if !self.cloud_endpoint.is_empty() {
            Some("custom")
        } else {
            None
        }
    }
    pub fn voice_enabled(&self) -> bool {
        self.voice != Some(false)
    }
    pub fn system_prompt(&self) -> &str {
        if self.personality.trim().is_empty() {
            include_str!("persona.txt").trim()
        } else {
            self.personality.trim()
        }
    }
    pub fn automatic_percent(ram: u64) -> u8 {
        if ram < 12 << 30 {
            25
        } else if ram < 32 << 30 {
            50
        } else {
            75
        }
    }
    /// Cofres da Athena, incluindo a pasta escolhida em versões anteriores.
    pub fn vaults(&self) -> Vec<String> {
        let mut out = self.athena_vaults.clone();
        if !self.athena_folder.is_empty() && !out.contains(&self.athena_folder) {
            out.insert(0, self.athena_folder.clone());
        }
        out
    }
    pub fn validate(&self) -> io::Result<()> {
        if !hex_color(&self.desktop.panel_color) || !hex_color(&self.desktop.panel_accent)
            || self.desktop.panel_radius > 48 || self.desktop.panel_font.chars().count() > 100
            || self.desktop.panel_font.chars().any(char::is_control)
            || self.personality.chars().count() > 4000
            || self.athena_folder.chars().count() > 4096
            || self.athena_folder.chars().any(char::is_control)
            || (!self.athena_folder.is_empty() && !std::path::Path::new(&self.athena_folder).is_absolute())
            || self.athena_vaults.len() > 50
            || self.athena_vaults.iter().any(|v| v.is_empty() || v.chars().count() > 4096 || v.chars().any(char::is_control) || !std::path::Path::new(v).is_absolute())
            || self.version != VERSION
            || self.percent() < 5
            || self.percent() > 90
            || self.history_messages < 2
            || self.history_messages > 200
            || self.history_chars < 1000
            || self.history_chars > 200000
            || self.context_tokens < 512
            || self.context_tokens > 32768
            || self.idle_seconds > 3600
        {
            return Err(io::Error::other("Configuração fora dos limites suportados"));
        }
        if !self.cloud_provider.is_empty() && self.cloud_provider != "custom" && super::cloud::provider(&self.cloud_provider).is_none() {
            return Err(io::Error::other("Plataforma de nuvem desconhecida"));
        }
        if !self.image_provider.is_empty() && super::cloud::provider(&self.image_provider).is_none_or(|p| p.images.is_none()) {
            return Err(io::Error::other("Esta plataforma não cria imagens"));
        }
        if self.cloud_model.chars().count() > 200 || self.image_model.chars().count() > 200 {
            return Err(io::Error::other("Nome de modelo longo demais"));
        }
        if !self.cloud_endpoint.is_empty() {
            let url = reqwest::Url::parse(&self.cloud_endpoint).map_err(io::Error::other)?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(io::Error::other(
                    "Provedor cloud deve usar uma URL HTTPS sem credenciais embutidas",
                ));
            }
        }
        Ok(())
    }
}

pub struct PreferenceStore {
    path: PathBuf,
}

impl PreferenceStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("preferences.json"),
        }
    }
    pub fn load(&self) -> io::Result<Preferences> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let mut value: Preferences =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                if value.version == 1 {
                    // O padrão antigo não é distinguível de uma escolha igual; demais valores ficam.
                    if value.context_tokens == V1_DEFAULT_CONTEXT {
                        value.context_tokens = Preferences::default().context_tokens;
                    }
                    value.version = VERSION;
                }
                value.validate()?;
                Ok(value)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preferences::initial()),
            Err(error) => Err(error),
        }
    }
    pub fn save(&self, value: &Preferences) -> io::Result<()> {
        value.validate()?;
        fs::create_dir_all(self.path.parent().unwrap())?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        file.sync_all()?;
        fs::rename(temporary, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_defaults_and_opt_out_survive_reopening() {
        let root = std::env::temp_dir().join(format!("ayla-desktop-prefs-{}", super::super::store::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("preferences.json"), r#"{"version":2}"#).unwrap();
        let store = PreferenceStore::new(&root);
        let mut prefs = store.load().unwrap();
        assert!(prefs.desktop.autostart && prefs.desktop.background && prefs.desktop.system_theme);
        prefs.desktop.autostart = false;
        prefs.desktop.background = false;
        prefs.desktop.system_theme = false;
        prefs.desktop.panel_radius = 0;
        store.save(&prefs).unwrap();
        assert_eq!(store.load().unwrap().desktop, prefs.desktop);
        prefs.desktop.panel_color = "invalid".into();
        assert!(store.save(&prefs).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn automatic_profile_adapts_to_hardware_without_overriding_manual_choices() {
        assert_eq!(Preferences::automatic_percent(8 << 30), 25);
        assert_eq!(Preferences::automatic_percent(16 << 30), 50);
        assert_eq!(Preferences::automatic_percent(32 << 30), 75);
        let mut prefs = Preferences::default();
        prefs.profile = Some(Profile::Light);
        assert_eq!(prefs.percent(), 25);
        prefs.resource_percent = Some(40);
        assert_eq!(prefs.percent(), 40);
    }
    #[test]
    fn personality_defaults_migrates_and_restores_without_affecting_privacy() {
        let mut prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(prefs.system_prompt(), include_str!("persona.txt").trim());
        prefs.personality = "Responda de forma breve e acolhedora.".into();
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.system_prompt(), prefs.personality);
        assert!(matches!(restored.usage_mode, UsageMode::LocalOnly));
        prefs.personality = " ".into();
        assert_eq!(prefs.system_prompt(), include_str!("persona.txt").trim());
        prefs.personality = "x".repeat(4001);
        assert!(prefs.validate().is_err());
    }
    #[test]
    fn first_run_does_not_require_or_write_configuration() {
        let dir = std::env::temp_dir().join(format!("ayla-preferences-{}", std::process::id()));
        let store = PreferenceStore::new(&dir);
        assert_eq!(store.load().unwrap(), Preferences::initial());
        assert!(!dir.exists());
    }
    #[test]
    fn configuration_round_trip_preserves_custom_limits_and_rejects_invalid_values() {
        let dir = std::env::temp_dir().join(format!(
            "ayla-preferences-roundtrip-{}",
            super::super::store::id()
        ));
        let store = PreferenceStore::new(&dir);
        let mut value = Preferences::initial();
        value.profile = Some(Profile::Light);
        value.resource_percent = Some(30);
        store.save(&value).unwrap();
        assert_eq!(store.load().unwrap(), value);
        value.resource_percent = Some(100);
        assert!(store.save(&value).is_err());
        assert_eq!(store.load().unwrap().resource_percent, Some(30));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn version_one_migrates_default_context_and_preserves_custom_values() {
        let dir = std::env::temp_dir().join(format!(
            "ayla-preferences-v1-{}",
            super::super::store::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let store = PreferenceStore::new(&dir);
        fs::write(
            &store.path,
            r#"{"version":1,"context_tokens":2048,"personality":"breve"}"#,
        )
        .unwrap();
        let migrated = store.load().unwrap();
        assert_eq!((migrated.version, migrated.context_tokens), (2, 8192));
        assert_eq!(migrated.personality, "breve");
        assert_eq!(migrated.notifications, Notifications { reply: true, setup: true, reminders: true });
        fs::write(&store.path, r#"{"version":1,"context_tokens":4096}"#).unwrap();
        assert_eq!(store.load().unwrap().context_tokens, 4096);
        // Carregar não reescreve o arquivo; a migração persiste no próximo salvamento.
        assert!(fs::read_to_string(&store.path).unwrap().contains("4096"));
        fs::write(&store.path, r#"{"version":3}"#).unwrap();
        assert!(store.load().is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn microphone_is_on_by_default_and_only_an_explicit_choice_turns_it_off() {
        assert!(Preferences::default().voice_enabled());
        let legacy: Preferences = serde_json::from_str(r#"{"version":2,"voice":null}"#).unwrap();
        assert!(legacy.voice_enabled(), "arquivos de antes da decisão ficam com o microfone ligado");
        let declined: Preferences = serde_json::from_str(r#"{"version":2,"voice":false}"#).unwrap();
        assert!(!declined.voice_enabled(), "quem desligou continua com ele desligado");
    }
    #[test]
    fn cloud_platform_is_validated_and_old_endpoints_still_work() {
        let mut prefs = Preferences::default();
        assert_eq!((prefs.cloud_platform(), prefs.cloud_asked), (None, false));
        prefs.cloud_endpoint = "https://exemplo.com/v1".into();
        assert_eq!(prefs.cloud_platform(), Some("custom"));
        prefs.cloud_provider = "anthropic".into();
        assert!(prefs.validate().is_ok());
        assert_eq!(prefs.cloud_platform(), Some("anthropic"));
        prefs.cloud_provider = "inventada".into();
        assert!(prefs.validate().is_err());
    }
    #[test]
    fn notifications_default_on_and_keep_individual_choices() {
        let dir = std::env::temp_dir().join(format!(
            "ayla-preferences-notify-{}",
            super::super::store::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let store = PreferenceStore::new(&dir);
        // Arquivo salvo antes das notificações existirem.
        fs::write(&store.path, r#"{"version":2,"voice":true}"#).unwrap();
        let mut prefs = store.load().unwrap();
        assert_eq!(prefs.notifications, Notifications::default());
        assert!(prefs.notifications.reply && prefs.notifications.setup);
        prefs.notifications.reply = false;
        store.save(&prefs).unwrap();
        let saved = store.load().unwrap().notifications;
        assert_eq!((saved.reply, saved.setup), (false, true));
        fs::write(&store.path, r#"{"version":2,"notifications":{"reply":true,"desconhecida":true}}"#).unwrap();
        assert!(store.load().is_err(), "campos desconhecidos continuam recusados");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn corrupted_configuration_is_not_silently_overwritten() {
        let dir =
            std::env::temp_dir().join(format!("ayla-preferences-corrupt-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let store = PreferenceStore::new(&dir);
        fs::write(&store.path, "invalid").unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read_to_string(&store.path).unwrap(), "invalid");
        fs::remove_dir_all(dir).unwrap();
    }
}
