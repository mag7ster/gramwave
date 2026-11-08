use crate::*;
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
fn phase(tg: bool, state: &Value) -> Phase {
    if !tg {
        return Phase::Setup;
    }
    match state["@type"].as_str().unwrap_or("") {
        "authorizationStateWaitPhoneNumber" | "authorizationStateWaitOtherDeviceConfirmation" => {
            Phase::Method
        }
        "authorizationStateWaitCode" | "authorizationStateWaitEmailCode" => Phase::Code,
        "authorizationStateWaitPassword" => Phase::Password,
        "authorizationStateWaitEmailAddress" => Phase::Email,
        "authorizationStateReady" => Phase::Ready,
        "authorizationStateClosed" => Phase::Closed,
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
    pub(crate) fn submit_auth(&mut self, v: Value) {
        if self.auth_pending {
            return;
        }
        let Some(tg) = self.tg.clone() else { return };
        self.auth_token += 1;
        let token = self.auth_token;
        self.auth_pending = true;
        self.auth_error.clear();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = tg.call(v).map(|_| ());
            let _ = tx.send(Event::AuthResult(token, result));
        });
    }
    pub(crate) fn auth_ui(&mut self, ui: &mut egui::Ui) {
        let current = match std::env::args()
            .find_map(|a| a.strip_prefix("--preview-auth=").map(str::to_owned))
            .as_deref()
        {
            Some("qr") => Phase::Method,
            Some("code") | Some("error") | Some("email-code") => Phase::Code,
            Some("password") => Phase::Password,
            Some("email") => Phase::Email,
            _ => phase(self.tg.is_some(), &self.auth),
        };

        let step = format!("{current:?}");
        let changed = step != self.last_auth_step;
        self.last_auth_step = step;
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let pending = self.auth_pending;
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
ui.add(egui::TextEdit::singleline(&mut self.state.api_id).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text("12345678"));
ui.label(tr("API hash"));
ui.add(egui::TextEdit::singleline(&mut self.state.api_hash).password(true).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY));
if ui.add_enabled(credentials(&self.state.api_id,&self.state.api_hash),egui::Button::new(egui::RichText::new(tr("Continue")).color(egui::Color32::BLACK)).fill(design::GREEN).corner_radius(20).min_size(egui::vec2(ui.available_width(),40.))).clicked()||(enter&&credentials(&self.state.api_id,&self.state.api_hash)){
self.state.api_id=self.state.api_id.trim().into();
self.state.api_hash=self.state.api_hash.trim().into();
self.connect();
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
self.submit_auth(json!({
"@type":"setAuthenticationPhoneNumber","phone_number":self.phone,"settings":null}
));
}
else{
self.auth_error=tr("Enter a phone number including the country code, for example +44…");
}
}
}
,
   Phase::Code=>{
ui.heading(tr("Check your messages"));
let delivery=self.auth["code_info"]["type"]["@type"].as_str().unwrap_or("");
ui.label(match delivery{
"authenticationCodeTypeTelegramMessage"=>tr("We sent a code to your Telegram app."),"authenticationCodeTypeSms"=>tr("We sent an SMS to your phone."),_=>tr("Enter the verification code sent by Telegram.")}
);
let response=ui.add(egui::TextEdit::singleline(&mut self.code).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text(tr("Verification code")));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Verify code")).clicked()||enter{
let email=self.auth["@type"]=="authorizationStateWaitEmailCode";
self.submit_auth(if email{
json!({
"@type":"checkAuthenticationEmailCode","code":{
"@type":"emailAddressAuthenticationCode","code":self.code}
}
)}
else{
json!({
"@type":"checkAuthenticationCode","code":self.code}
)}
);
}
let wait=self.resend_at.map(|t|t.saturating_duration_since(std::time::Instant::now()).as_secs()).unwrap_or(0);
if self.auth["code_info"]["next_type"].is_object(){
if wait>0{
ui.weak(tr("Resend available in {seconds}s").replace("{seconds}",&wait.to_string()));
}
else if ui.button(tr("Resend code")).clicked(){
self.submit_auth(json!({
"@type":"resendAuthenticationCode","reason":null}
));
}
}
}
,
   Phase::Password=>{
ui.heading(tr("Two-step verification"));
ui.label(tr("Enter your Telegram password to finish signing in."));
ui.weak(self.auth["password_hint"].as_str().unwrap_or(""));
let response=ui.add(egui::TextEdit::singleline(&mut self.password).password(true).margin(egui::vec2(12.,10.)).desired_width(f32::INFINITY).hint_text(tr("Password")));
if changed{
response.request_focus();
}
if design::primary(ui,tr("Sign in")).clicked()||enter{
self.submit_auth(json!({
"@type":"checkAuthenticationPassword","password":self.password}
));
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
self.submit_auth(json!({
"@type":"setAuthenticationEmailAddress","email_address":self.phone}
));
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
            ui.colored_label(egui::Color32::from_rgb(255, 120, 120), &self.auth_error);
        }
        ui.add_space(18.);
        ui.weak(tr("Listening does not announce you online."));
        if self.tg.is_some() && !pending && ui.button(tr("Start over")).clicked() {
            self.submit_auth(json!({"@type":"close"}));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_validation() {
        assert!(credentials("123", "0123456789abcdef0123456789abcdef"));
        assert!(!credentials("0", "0123456789abcdef0123456789abcdef"));
        assert!(!credentials("123", "wrong"));
    }
    #[test]
    fn qr_can_require_password() {
        assert_eq!(
            phase(true, &json!({"@type":"authorizationStateWaitPassword"})),
            Phase::Password
        );
    }
}
