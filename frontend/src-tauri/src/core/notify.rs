//! Notificações do sistema. Só avisam com a janela da Ayla fora de foco e
//! nunca incluem o conteúdo de conversas: a área de notificações fica visível
//! para quem estiver diante da tela e pode guardar um histórico.
use super::preferences::Preferences;
use tauri_plugin_notification::NotificationExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Uma resposta terminou.
    Reply,
    /// O preparo de modelos terminou.
    SetupDone,
    /// O preparo de modelos parou por erro (não por cancelamento).
    SetupFailed,
}

impl Event {
    fn enabled(self, prefs: &Preferences) -> bool {
        match self {
            Event::Reply => prefs.notifications.reply,
            Event::SetupDone | Event::SetupFailed => prefs.notifications.setup,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Event::Reply => "Sua resposta está pronta.",
            Event::SetupDone => "Tudo pronto: a Ayla terminou de se preparar neste computador.",
            Event::SetupFailed => "O preparo não terminou. Abra a Ayla para tentar de novo.",
        }
    }
}

/// Avisa só quando o tipo está ligado e a pessoa não está olhando para a Ayla.
pub fn should_notify(event: Event, prefs: &Preferences, focused: bool) -> bool {
    !focused && event.enabled(prefs)
}

/// Botão "Testar notificação" em Configurações: avisa na hora, com ou sem foco.
#[tauri::command]
pub fn notify_test(app: tauri::AppHandle) -> Result<(), String> {
    app.notification()
        .builder()
        .title("Ayla")
        .body("As notificações estão funcionando.")
        .show()
        .map_err(|_| "O sistema recusou a notificação".to_string())
}

/// Lembrete na hora marcada: o texto é do próprio lembrete; avisa mesmo com a Ayla em foco.
pub fn reminder(app: &tauri::AppHandle, prefs: &Preferences, text: &str, late: bool) {
    if !prefs.notifications.reminders {
        return;
    }
    let title = if late { "Lembrete atrasado" } else { "Lembrete" };
    let _ = app.notification().builder().title(title).body(text).show();
}

pub fn send(app: &tauri::AppHandle, prefs: &Preferences, event: Event) {
    let focused = crate::desktop::ayla_focused(app);
    if !should_notify(event, prefs, focused) {
        return;
    }
    let _ = app.notification().builder().title("Ayla").body(event.text()).show();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifies_only_enabled_events_while_the_window_is_away() {
        let mut prefs = Preferences::default();
        for event in [Event::Reply, Event::SetupDone, Event::SetupFailed] {
            assert!(should_notify(event, &prefs, false), "ligadas por padrão: {event:?}");
            assert!(!should_notify(event, &prefs, true), "com a janela em foco não avisa: {event:?}");
        }
        prefs.notifications.reply = false;
        assert!(!should_notify(Event::Reply, &prefs, false));
        assert!(should_notify(Event::SetupDone, &prefs, false));
        prefs.notifications.setup = false;
        assert!(!should_notify(Event::SetupDone, &prefs, false));
        assert!(!should_notify(Event::SetupFailed, &prefs, false));
    }
}
