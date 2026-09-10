use fusion::provider::types::{ImageAttachment, Message, Role};

#[test]
fn test_loop_runner_message_with_image() {
    let img = ImageAttachment {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="
            .to_string(),
        path: Some(".fusion/cache/images/clip_test.png".to_string()),
        width: Some(1),
        height: Some(1),
    };
    let msg = Message::user_with_images("Describe this screenshot", vec![img]);
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.content, "Describe this screenshot");
    assert!(msg.images.is_some());
    let imgs = msg.images.as_ref().unwrap();
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].width, Some(1));
    assert_eq!(imgs[0].height, Some(1));
}

#[test]
fn test_encode_and_attach_roundtrip() {
    let rgba_data = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
    let png_bytes = fusion::ui::clipboard_image::encode_rgba_png(2, 2, &rgba_data)
        .expect("must encode valid PNG");
    assert!(!png_bytes.is_empty());

    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("fusion_test_roundtrip.png");
    std::fs::write(&temp_file, &png_bytes).expect("must write temp file");

    let att = fusion::ui::clipboard_image::create_image_attachment_from_file(&temp_file)
        .expect("must create attachment");
    assert_eq!(att.width, Some(2));
    assert_eq!(att.height, Some(2));
    assert_eq!(att.media_type, "image/png");
    assert!(!att.data.is_empty());

    let _ = std::fs::remove_file(temp_file);
}
