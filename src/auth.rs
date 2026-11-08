use crate::{design, tr, App};
use eframe::egui;
use tg_player_application::telegram::{AuthRequest, AuthorizationState};
#[derive(Debug, PartialEq)]
enum Phase {
    Setup,
    Starting,
    Method,
    Code,
    Password,
    Email,
    Ready,
    Closed,
}
fn phase(tg: bool, state: &AuthorizationState) -> Phase {
    if !tg {
        return Phase::Setup;
    }
    match state {
        AuthorizationState::Phone | AuthorizationState::Confirmation(_) => Phase::Method,
        AuthorizationState::Code { .. } => Phase::Code,
        AuthorizationState::Password(_) => Phase::Password,
        AuthorizationState::Email => Phase::Email,
        AuthorizationState::Ready => Phase::Ready,
        AuthorizationState::Closed => Phase::Closed,
        _ => Phase::Starting,
    }
}
fn credentials(id: &str, hash: &str) -> bool {
    id.trim().parse::<i32>().is_ok_and(|n| n > 0)
        && hash.trim().len() == 32
        && hash.trim().bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn friendly_error(error: &str) -> String {
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
            "API_ID_INVALID",
            "Check the API ID and hash from my.telegram.org.",
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
        self.core_command(tg_player_application::commands::AppCommand::Authenticate(
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
        ui.add_enabled_ui(!pending,|ui|{match current{
   Phase::Setup=>{
ui.heading(tr("Connect your Telegram"));
ui.label(egui::RichText::new(tr("ONE-TIME SETUP")).small().color(design::GREEN));
ui.weak(tr("Register an app at my.telegram.org, then paste its credentials below. Your login is saved on this device."));
ui.horizontal(|ui| {
    ui.hyperlink_to(tr("Get application credentials"),"https://my.telegram.org/apps");
    if ui.small_button(tr("Copy link")).clicked(){ui.ctx().copy_text("https://my.telegram.org/apps".into());}
});
ui.add_space(12.);
ui.label(tr("API ID"));
ui.add(egui::TextEdit::singleline(&mut self.api_id_input).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text("12345678"));
ui.label(tr("API hash"));
ui.add(egui::TextEdit::singleline(&mut self.api_hash_input).password(true).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY));
if ui.add_enabled(credentials(&self.api_id_input,&self.api_hash_input),egui::Button::new(egui::RichText::new(tr("Continue")).color(egui::Color32::BLACK)).fill(design::GREEN).corner_radius(20).min_size(egui::vec2(ui.available_width(),40.))).clicked()||(enter&&credentials(&self.api_id_input,&self.api_hash_input)){
self.api_id_input=self.api_id_input.trim().into();
self.api_hash_input=self.api_hash_input.trim().into();
let mut settings=tg_player_application::persistence::Settings::from(self.state.view());
settings.api_id=self.api_id_input.clone();settings.api_hash=self.api_hash_input.clone();
self.execute(crate::application::commands::AppCommand::Settings(settings));
self.execute(crate::application::commands::AppCommand::Connect);
self.persist();
}
}
,
   Phase::Method=>{
ui.heading(tr("Sign in to Telegram"));
if let Ok(code)=qrcode::QrCode::new(self.qr.as_bytes()){
if !self.qr.is_empty(){
let size=240.;
let width=code.width();
let cell=size/(width+8)as f32;
let(rect,_)=ui.allocate_exact_size(egui::vec2(size,size),egui::Sense::hover());
ui.painter().rect_filled(rect,8.,egui::Color32::WHITE);
for y in 0..width{
for x in 0..width{
if code[(x,y)]==qrcode::Color::Dark{
ui.painter().rect_filled(egui::Rect::from_min_size(rect.min+egui::vec2((x+4)as f32*cell,(y+4)as f32*cell),egui::vec2(cell,cell)),0.,egui::Color32::BLACK);
}
}
}
}
}
ui.label(tr("Telegram → Settings → Devices → Link Desktop Device"));
ui.add_space(12.);
ui.separator();
ui.label(tr("Or sign in with your phone"));
let response=ui.add(egui::TextEdit::singleline(&mut self.phone).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text(tr("Phone number with country code")));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Continue with phone")).clicked()||enter{
if self.phone.starts_with('+')&&self.phone.chars().filter(char::is_ascii_digit).count()>=7{
self.execute(crate::application::commands::AppCommand::Authenticate(AuthRequest::Phone(self.phone.clone())));
}
else{
self.auth_error=tr("Enter a phone number including the country code, for example +44…");
}
}
}
,
   Phase::Code=>{
ui.heading(tr("Check your messages"));
let delivery=self.state.auth().state().delivery();
ui.label(match delivery{
"authenticationCodeTypeTelegramMessage"=>tr("We sent a code to your Telegram app."),"authenticationCodeTypeSms"=>tr("We sent an SMS to your phone."),_=>tr("Enter the verification code sent by Telegram.")}
);
let response=ui.add(egui::TextEdit::singleline(&mut self.code).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text(tr("Verification code")));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Verify code")).clicked()||enter{
let email=self.state.auth().state().email_code();
self.execute(crate::application::commands::AppCommand::Authenticate(if email {AuthRequest::EmailCode(self.code.clone())}else{AuthRequest::Code(self.code.clone())}));
}
let wait=self.resend_at.map(|t|t.saturating_duration_since(std::time::Instant::now()).as_secs()).unwrap_or(0);
if self.state.auth().state().can_resend(){
if wait>0{
ui.weak(tr("Resend available in {seconds}s").replace("{seconds}",&wait.to_string()));
}
else if ui.button(tr("Resend code")).clicked(){
self.execute(crate::application::commands::AppCommand::Authenticate(AuthRequest::Resend));
}
}
}
,
   Phase::Password=>{
ui.heading(tr("Two-step verification"));
ui.label(tr("Enter your Telegram password to finish signing in."));
ui.weak(self.state.auth().state().password_hint());
let response=ui.add(egui::TextEdit::singleline(&mut self.password).password(true).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text(tr("Password")));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Sign in")).clicked()||enter{
self.execute(crate::application::commands::AppCommand::Authenticate(AuthRequest::Password(self.password.clone())));
}
}
,
   Phase::Email=>{
ui.heading(tr("Verify your email"));
let response=ui.add(egui::TextEdit::singleline(&mut self.phone).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text("name@example.com"));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Continue")).clicked()||enter{
self.execute(crate::application::commands::AppCommand::Authenticate(AuthRequest::Email(self.phone.clone())));
}
}
,
   Phase::Closed=>{ui.label(tr("Connection closed. Please reconnect."));},Phase::Starting=>{ui.spinner();ui.label(tr("Connecting securely to Telegram…"));},Phase::Ready=>{}
  }});
        if pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Please wait…"));
            });
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
