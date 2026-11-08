use eframe::egui;
fastframe_icons::icons! {
 pub enum Icon {
prefix:"tgplayer-",directory:"../assets/",Play=>lucide "play",Pause=>lucide "pause",Next => "next",Previous => "previous",More=>lucide "ellipsis",Download => "download",Queue => "queue",Settings=>lucide "settings",Search=>lucide "search",Close=>lucide "x",Shuffle => "shuffle",Repeat => "repeat"}
 }

pub const GREEN: egui::Color32 = egui::Color32::from_rgb(30, 215, 96);
pub fn install(ctx: &egui::Context) {
    fastframe_icons::install::<Icon>(ctx);
}
pub fn style(ctx: &egui::Context, light: bool) {
    let mut v = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    if !light {
        v.panel_fill = egui::Color32::from_rgb(18, 18, 18);
        v.window_fill = egui::Color32::from_rgb(26, 26, 26);
        v.extreme_bg_color = egui::Color32::from_rgb(35, 35, 35);
        v.override_text_color = Some(egui::Color32::from_rgb(245, 245, 245));
        v.weak_text_color = Some(egui::Color32::from_gray(165));
        v.widgets.inactive.bg_fill = egui::Color32::from_rgb(38, 38, 38);
    }
    v.selection.bg_fill = egui::Color32::from_rgb(30, 85, 48);
    ctx.set_visuals(v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(12., 10.);
        s.spacing.button_padding = egui::vec2(12., 8.);
        s.spacing.interact_size.y = 32.;
    });
}
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &'static str) -> bool {
    ui.add(
        egui::Button::image(icon.image(ui.visuals().text_color(), 20.))
            .frame(false)
            .min_size(egui::vec2(34., 34.)),
    )
    .on_hover_text(crate::tr(label))
    .clicked()
}
