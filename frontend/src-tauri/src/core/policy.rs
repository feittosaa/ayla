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
    long_identifier
        || text.contains('@')
        || [
            "senha",
            "password",
            "token",
            "secret",
            "sk-",
            "data de nascimento",
            "cpf",
            "cnpj",
            "endereço",
            "endereco",
            "telefone",
            "cartão",
            "cartao",
            "banco",
            "saldo",
            "salário",
            "salario",
            "diagnóstico",
            "diagnostico",
            "prontuário",
            "prontuario",
            "chave privada",
        ]
        .iter()
        .any(|pattern| lower.contains(pattern))
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
