use eframe::egui;
#[test]
fn jpeg_png_webp_decode_through_real_egui_loader() {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        4,
        4,
        image::Rgb([25, 160, 100]),
    ));
    for (format, name) in [
        (image::ImageFormat::Jpeg, "jpeg"),
        (image::ImageFormat::Png, "png"),
        (image::ImageFormat::WebP, "webp"),
    ] {
        let dir = std::env::temp_dir().join(format!("tg-player-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("avatar # музыка {name}"));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        std::fs::write(&path, bytes.into_inner()).unwrap();
        let uri = format!("file://{}", path.display());
        let mut ready = false;
        for _ in 0..100 {
            match ctx.try_load_image(&uri, egui::load::SizeHint::Scale(1_f32.into())) {
                Ok(egui::load::ImagePoll::Ready { image }) => {
                    assert_eq!(image.size, [4, 4]);
                    ready = true;
                    break;
                }
                Ok(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                Err(e) => panic!("{name}: {e}"),
            }
        }
        assert!(ready, "{name} never decoded");
        ctx.forget_image(&uri);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
