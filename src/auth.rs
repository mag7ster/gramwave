use crate::{design, tr, App};
use eframe::egui;
use gramwave_application::telegram::{AuthRequest, AuthorizationState};

pub(crate) const TELEGRAM_CREDENTIALS: Option<(i32, &str)> =
    include!(concat!(env!("OUT_DIR"), "/telegram_credentials.rs"));

#[derive(Debug, PartialEq)]
enum Phase {
    Disconnected,
    Starting,
    Method,
    Code,
    Password,
    Email,
    Ready,
}
fn phase(tg: bool, state: &AuthorizationState) -> Phase {
    if !tg {
        return Phase::Disconnected;
    }
    match state {
        AuthorizationState::Phone | AuthorizationState::Confirmation(_) => Phase::Method,
        AuthorizationState::Code { .. } => Phase::Code,
        AuthorizationState::Password(_) => Phase::Password,
        AuthorizationState::Email => Phase::Email,
        AuthorizationState::Ready => Phase::Ready,
        AuthorizationState::Closed => Phase::Disconnected,
        _ => Phase::Starting,
    }
}
pub(crate) fn friendly_error(error: &str) -> String {
    if error.contains("API_ID_INVALID") {
        return tr("Telegram rejected this build’s API credentials.");
    }
    for (code, message) in [
        (
            "PHONE_NUMBER_INVALID",
            "Check your phone number and country code.",
        ),
        (
            "PHONE_CODE_INVALID",
            "That code is incorrect. Please try again.",
        ),
        (
            "PHONE_CODE_EXPIRED",
            "This code has expired. Request a new code.",
        ),
        (
            "PASSWORD_HASH_INVALID",
            "That password is incorrect. Please try again.",
        ),
        (
            "EMAIL_CODE_INVALID",
            "That email code is incorrect. Please try again.",
        ),
    ] {
        if error.contains(code) {
            return tr(message);
        }
    }
    error.into()
}
impl App {
    pub(crate) fn submit_auth(&mut self, request: AuthRequest) {
        self.auth_error.clear();
        self.core_command(gramwave_application::commands::AppCommand::Authenticate(
            request,
        ));
    }
    pub(crate) fn auth_ui(&mut self, ui: &mut egui::Ui) {
        let current = match self.options.preview_auth.as_deref() {
            Some("qr") => Phase::Method,
            Some("code") | Some("error") | Some("email-code") => Phase::Code,
            Some("password") => Phase::Password,
            Some("email") => Phase::Email,
            _ => phase(self.tg.is_some(), self.state.auth().state()),
        };

        let step = format!("{current:?}");
        let changed = step != self.last_auth_step;
        self.last_auth_step = step;
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let pending = self.state.auth().running();
        ui.add_enabled_ui(!pending, |ui| match current {
            Phase::Disconnected => {
                ui.heading(tr("Sign in to Telegram"));
                if TELEGRAM_CREDENTIALS.is_some() {
                    if design::primary(ui, tr("Reconnect")).clicked() || enter {
                        self.execute(crate::application::commands::AppCommand::Connect);
                    }
                } else {
                    if cfg!(feature = "telegram") {
                        ui.label(tr("This build is missing Telegram credentials."));
                    } else {
                        ui.label(tr("This build does not include Telegram support."));
                    }
                    ui.hyperlink_to(
                        tr("Build instructions"),
                        "https://github.com/mag7ster/gramwave#build-and-run",
                    );
                }
            }
            Phase::Method => {
                ui.heading(tr("Sign in to Telegram"));
                if let Ok(code) = qrcode::QrCode::new(self.qr.as_bytes()) {
                    if !self.qr.is_empty() {
                        let size = ui.available_width().min(240.);
                        let width = code.width();
                        let cell = size / (width + 8) as f32;
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 8., egui::Color32::WHITE);
                        for y in 0..width {
                            for x in 0..width {
                                if code[(x, y)] == qrcode::Color::Dark {
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            rect.min
                                                + egui::vec2(
                                                    (x + 4) as f32 * cell,
                                                    (y + 4) as f32 * cell,
                                                ),
                                            egui::vec2(cell, cell),
                                        ),
                                        0.,
                                        egui::Color32::BLACK,
                                    );
                                }
                            }
                        }
                    }
                }
                ui.label(tr("Telegram → Settings → Devices → Link Desktop Device"));
                ui.add_space(12.);
                ui.separator();
                ui.label(tr("Or sign in with your phone"));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.phone)
                        .margin(egui::vec2(12., 10.))
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("Phone number with country code")),
                );
                if changed {
                    response.request_focus();
                }
                if design::primary(ui, tr("Continue with phone")).clicked() || enter {
                    if self.phone.starts_with('+')
                        && self.phone.chars().filter(char::is_ascii_digit).count() >= 7
                    {
                        self.execute(crate::application::commands::AppCommand::Authenticate(
                            AuthRequest::Phone(self.phone.clone()),
                        ));
                    } else {
                        self.auth_error =
                            tr("Enter a phone number including the country code, for example +44…");
                    }
                }
            }
            Phase::Code => {
                ui.heading(tr("Check your messages"));
                let delivery = self.state.auth().state().delivery();
                ui.label(match delivery {
                    "authenticationCodeTypeTelegramMessage" => {
                        tr("We sent a code to your Telegram app.")
                    }
                    "authenticationCodeTypeSms" => tr("We sent an SMS to your phone."),
                    _ => tr("Enter the verification code sent by Telegram."),
                });
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.code)
                        .margin(egui::vec2(12., 10.))
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("Verification code")),
                );
                if changed {
                    response.request_focus();
                }
                if design::primary(ui, tr("Verify code")).clicked() || enter {
                    let email = self.state.auth().state().email_code();
                    self.execute(crate::application::commands::AppCommand::Authenticate(
                        if email {
                            AuthRequest::EmailCode(self.code.clone())
                        } else {
                            AuthRequest::Code(self.code.clone())
                        },
                    ));
                }
                let wait = self
                    .resend_at
                    .map(|t| {
                        t.saturating_duration_since(std::time::Instant::now())
                            .as_secs()
                    })
                    .unwrap_or(0);
                if self.state.auth().state().can_resend() {
                    if wait > 0 {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(1));
                        ui.weak(
                            tr("Resend available in {seconds}s")
                                .replace("{seconds}", &wait.to_string()),
                        );
                    } else if ui.button(tr("Resend code")).clicked() {
                        self.execute(crate::application::commands::AppCommand::Authenticate(
                            AuthRequest::Resend,
                        ));
                    }
                }
            }
            Phase::Password => {
                ui.heading(tr("Two-step verification"));
                ui.label(tr("Enter your Telegram password to finish signing in."));
                ui.weak(self.state.auth().state().password_hint());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.password)
                        .password(true)
                        .margin(egui::vec2(12., 10.))
                        .desired_width(f32::INFINITY)
                        .hint_text(tr("Password")),
                );
                if changed {
                    response.request_focus();
                }
                if design::primary(ui, tr("Sign in")).clicked() || enter {
                    self.execute(crate::application::commands::AppCommand::Authenticate(
                        AuthRequest::Password(self.password.clone()),
                    ));
                }
            }
            Phase::Email => {
                ui.heading(tr("Verify your email"));
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.phone)
                        .margin(egui::vec2(12., 10.))
                        .desired_width(f32::INFINITY)
                        .hint_text("name@example.com"),
                );
                if changed {
                    response.request_focus();
                }
                if design::primary(ui, tr("Continue")).clicked() || enter {
                    self.execute(crate::application::commands::AppCommand::Authenticate(
                        AuthRequest::Email(self.phone.clone()),
                    ));
                }
            }
            Phase::Starting => {
                ui.label(tr("Connecting securely to Telegram…"));
            }
            Phase::Ready => {}
        });
        if pending {
            ui.label(tr("Please wait…"));
        }
        if !self.auth_error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &self.auth_error);
            if ui.button(tr("Copy error")).clicked() {
                ui.ctx().copy_text(self.auth_error.clone());
            }
        }
        ui.add_space(18.);
        ui.weak(tr("Listening does not announce you online."));
        if self.tg.is_some() && !pending && ui.button(tr("Start over")).clicked() {
            self.execute(crate::application::commands::AppCommand::Authenticate(
                AuthRequest::Close,
            ));
        }
    }
}
#[cfg(test)]
#[path = "auth/tests.rs"]
mod tests;
