use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageMode {
    #[default]
    LocalOnly,
    CloudOnly,
    Hybrid,
    PerConversation,
    ExplicitRequest,
    Suggest,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Sensitive,
    #[default]
    Uncertain,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CloudDecision {
    pub allowed: bool,
    pub reason: String,
}

pub(super) fn suspect_content(text: &str) -> bool {
    suspect_reason(text).is_some()
}

/// Marcas inequívocas de dado real num texto longo, como uma nota: pelo formato, nunca por palavras.
/// Número longo (documento, telefone, cartão), e-mail, chave de API e bloco de chave privada. Palavras
/// que numa história são ficção ("a senha da porta mágica", "o cartão do banco dos piratas", "a chave
/// privada do rei") ficam para o modelo local, que lê a nota inteira.
pub(super) fn certain_reason(text: &str) -> Option<String> {
    if suspect_reason(text).is_some_and(|reason| reason.starts_with("um número")) {
        return Some("um número longo, como documento, telefone ou cartão".into());
    }
    let words = || text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\'' | ','));
    if words().any(|word| word.split_once('@').is_some_and(|(user, domain)| !user.is_empty() && domain.contains('.') && !domain.ends_with('.'))) {
        return Some("um e-mail".into());
    }
    let key = |word: &str| ["sk-", "ghp_", "github_pat_", "xoxb-", "AKIA", "AIza"].iter()
        .any(|prefix| word.strip_prefix(prefix).is_some_and(|rest| rest.len() >= 16 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')));
    if words().any(key) {
        return Some("uma chave de acesso".into());
    }
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return Some("uma chave privada".into());
    }
    None
}

/// O que fez o texto parecer sensível (para explicar por que algo ficou neste computador).
pub(super) fn suspect_reason(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    // Barreiras adicionais; nunca classificar automaticamente como público.
    let mut digits = 0;
    let long_identifier = text.chars().any(|c| {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c != '.' && c != '-' && c != ' ' {
            digits = 0;
        }
        digits >= 8
    });
    if long_identifier {
        return Some("um número longo, como documento ou telefone".into());
    }
    if text.contains('@') {
        return Some("um e-mail ou @".into());
    }
    [
        "senha", "password", "token", "secret", "sk-", "data de nascimento", "cpf", "cnpj", "endereço", "endereco",
        "telefone", "cartão", "cartao", "banco", "saldo", "salário", "salario", "diagnóstico", "diagnostico",
        "prontuário", "prontuario", "chave privada",
    ]
    .iter()
    .find(|pattern| lower.contains(*pattern))
    .map(|pattern| format!("a palavra \"{pattern}\""))
}

pub fn cloud_gate(
    mode: UsageMode,
    sensitivity: Sensitivity,
    reviewed: bool,
    conversation_consent: bool,
    explicit: bool,
    text: &str,
) -> CloudDecision {
    let reason = if sensitivity != Sensitivity::Public || suspect_content(text) {
        Some("Conteúdo sensível ou duvidoso permanece local. Remova ou anonimize os dados antes de usar cloud.")
    } else if mode == UsageMode::LocalOnly {
        Some("O modo somente local bloqueia cloud.")
    } else if !reviewed {
        Some("Revise o conteúdo exato e confirme que é público antes do envio.")
    } else if mode == UsageMode::PerConversation && !conversation_consent {
        Some("Esta conversa ainda não permite cloud.")
    } else if matches!(mode, UsageMode::ExplicitRequest | UsageMode::Suggest) && !explicit {
        Some("O envio para cloud precisa ser solicitado explicitamente.")
    } else {
        None
    };
    CloudDecision {
        allowed: reason.is_none(),
        reason: reason.unwrap_or("Envio público autorizado").to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nota_de_ficcao_so_para_nas_marcas_certas() {
        assert_eq!(certain_reason("O capitão mostra um cartão do banco dos piratas e um token mágico."), None);
        assert!(certain_reason("Contato: rafa.mesa@exemplo.com").is_some());
        assert!(certain_reason("Telefone 11 98765-4321").is_some());
        assert_eq!(certain_reason("A senha da porta mágica era abre-te sésamo; a chave privada do rei sumiu."), None);
        assert!(certain_reason("chave: sk-proj4f9KdLq2Zx8Wm3Nv7Rt").is_some());
        assert!(certain_reason("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----").is_some());
        assert_eq!(certain_reason("sk-tiny e o e-mail rafa@ sem domínio"), None);
        assert_eq!(certain_reason("Sara Salamandra @ forja"), None, "@ solto não é e-mail");
    }

    use super::*;
    #[test]
    fn no_mode_overrides_sensitive_or_uncertain_content() {
        for mode in [
            UsageMode::LocalOnly,
            UsageMode::CloudOnly,
            UsageMode::Hybrid,
            UsageMode::PerConversation,
            UsageMode::ExplicitRequest,
            UsageMode::Suggest,
        ] {
            for sensitivity in [Sensitivity::Sensitive, Sensitivity::Uncertain] {
                assert!(!cloud_gate(mode, sensitivity, true, true, true, "texto").allowed);
            }
        }
    }
    #[test]
    fn public_label_does_not_override_sensitive_patterns_or_skip_review() {
        for text in [
            "meu CPF",
            "saldo banco",
            "user@example.com",
            "12345678900",
            "senha abc",
        ] {
            assert!(
                !cloud_gate(
                    UsageMode::CloudOnly,
                    Sensitivity::Public,
                    true,
                    true,
                    true,
                    text
                )
                .allowed
            );
        }
        assert!(
            !cloud_gate(
                UsageMode::CloudOnly,
                Sensitivity::Public,
                false,
                true,
                true,
                "Explique fotossíntese"
            )
            .allowed
        );
    }
    #[test]
    fn conversation_and_explicit_permissions_are_independent() {
        let text = "Explique fotossíntese";
        assert!(
            !cloud_gate(
                UsageMode::PerConversation,
                Sensitivity::Public,
                true,
                false,
                true,
                text
            )
            .allowed
        );
        assert!(
            !cloud_gate(
                UsageMode::Suggest,
                Sensitivity::Public,
                true,
                true,
                false,
                text
            )
            .allowed
        );
        assert!(
            cloud_gate(
                UsageMode::CloudOnly,
                Sensitivity::Public,
                true,
                false,
                false,
                text
            )
            .allowed
        );
    }
}
