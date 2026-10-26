use serde_json::json;

#[test]
fn render_lang_en_vi() {
    use wishly_api::notify::{render_lang, Lang};
    let p = json!({});
    let (s, b) = render_lang(Lang::En, "event.reminder", "List", &p);
    assert!(s.contains("is coming up") && b.contains("/dashboard"));
    let (s, _) = render_lang(Lang::Vi, "event.reminder", "List", &p);
    assert!(s.contains("Sắp đến ngày sự kiện"));
    assert_eq!(Lang::parse("vi-VN,vi;q=0.9"), Lang::Vi);
    assert_eq!(Lang::parse("en-US"), Lang::En);
    assert_eq!(Lang::parse("id-ID"), Lang::Id);
    assert_eq!(Lang::Id.code(), "id");
    assert_eq!(Lang::parse("fr"), Lang::Zh);
    assert_eq!(
        render_lang(Lang::Zh, "event.reminder", "清單", &p),
        wishly_api::notify::render("event.reminder", "清單", &p)
    );
}
