//! The strip: what it reads out, what lights, and what it sheds when narrow.

use super::*;

/// The strip's pills read out state rather than menu names.
#[test]
fn the_strip_shows_the_deck_not_the_word_emulation() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&MenuModel {
        roots: vec![MenuNode::submenu("Emulation", vec![])
            .with_status("CDJ-3000", MenuIcon::Emulation)
            .with_detail("3.20")],
        ..Default::default()
    });
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(drawn.iter().any(|t| t.contains("CDJ-3000")), "{drawn:?}");
    assert!(drawn.iter().any(|t| t.contains("3.20")), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t == "Emulation"), "{drawn:?}");
}

/// A notice adds its glyph to the strip; without one, nothing is drawn.
#[test]
fn a_notice_puts_its_badge_on_the_strip() {
    let ctx = test_ctx();
    let shapes = |notice: Option<Notice>| {
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![
                MenuNode::submenu("Instances", vec![]).with_status("Slot 1", MenuIcon::Instances)
            ],
            notice,
        });
        run(&ctx, Default::default(), |ui| p.draw(ui)).shapes.len()
    };
    let flagged = shapes(Some(Notice {
        title: "Software emulation (TCG)".into(),
        body: vec!["why".into()],
    }));
    assert!(flagged > shapes(None), "the notice drew nothing");
}

/// The strip reports itself as chrome, so a whole-window pass can skip it.
#[test]
fn the_strip_is_chrome() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&model());
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
    assert!(
        p.chrome()
            .iter()
            .any(|r| (r.height() - BAR_HEIGHT).abs() < 0.5),
        "{:?}",
        p.chrome()
    );
}

/// Hovering lights a strip button and a menu row, as seen in the painted
/// shapes.
#[test]
fn hovering_lights_the_thing_under_the_pointer() {
    // A menu Area fades in, so what reaches the shape list is the plate
    // multiplied by the animation's opacity. Compare loosely.
    fn filled(out: &egui::FullOutput, want: Color32) -> bool {
        out.shapes.iter().any(|c| match &c.shape {
            egui::Shape::Rect(r) => {
                let a = r.fill.a() as f32 / 255.0;
                a > 0.5
                    && [0, 1, 2].iter().all(|&i| {
                        let got = r.fill.to_array()[i] as f32 / a;
                        (got - want.to_array()[i] as f32).abs() <= 6.0
                    })
            }
            _ => false,
        })
    }

    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&model());

    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    assert!(!filled(&out, STRIP_HOVER), "lit with the pointer elsewhere");

    let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
    let _ = run(&ctx, hover(burger), |ui| p.draw(ui));
    let out = run(&ctx, hover(burger), |ui| p.draw(ui));
    assert!(filled(&out, STRIP_HOVER), "the sandwich did not light up");

    // Open it, then hover its first row.
    let _ = run(&ctx, click_at(burger), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let row = texts(&out)
        .into_iter()
        .find(|(t, _)| t == "Quit")
        .map(|(_, r)| r)
        .expect("the sandwich did not list Quit");
    // The panel fades in over several frames before its rows read as lit.
    for _ in 0..10 {
        let _ = run(&ctx, hover(row.center()), |ui| p.draw(ui));
    }
    let out = run(&ctx, hover(row.center()), |ui| p.draw(ui));
    assert!(filled(&out, ROW_HOVER), "the row did not light up");
}

/// Every strip button lights under the pointer, not just some of them.
#[test]
fn every_strip_button_lights() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&MenuModel {
        roots: vec![
            MenuNode::submenu(
                "Emulation",
                vec![MenuNode::item_enabled(
                    MenuId::Restart,
                    "Restart Emulation",
                    true,
                )],
            )
            .with_status("CDJ-3000", MenuIcon::Emulation),
            MenuNode::submenu("View", vec![]).with_icon(MenuIcon::View),
        ],
        ..Default::default()
    });
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));

    // Hover every point along the strip, so each button is found wherever it
    // is laid out.
    let mut lit_rects: Vec<Rect> = Vec::new();
    let mut x = 18.0;
    while x < 890.0 {
        let at = Pos2::new(x, BAR_HEIGHT / 2.0);
        let _ = run(&ctx, hover(at), |ui| p.draw(ui));
        let out = run(&ctx, hover(at), |ui| p.draw(ui));
        if let Some(r) = out.shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.fill == STRIP_HOVER && r.rect.contains(at) => Some(r.rect),
            _ => None,
        }) {
            if !lit_rects.iter().any(|k| (k.left() - r.left()).abs() < 1.0) {
                lit_rects.push(r);
            }
        }
        x += 2.0;
    }
    // The sandwich, the Emulation pill, Restart and View light up. Where the
    // strip is the title bar, minimise and maximise light up too; close
    // lights red, so it is not counted.
    let caption_buttons = if CAPTION_IN_STRIP { 2 } else { 0 };
    assert_eq!(
        lit_rects.len(),
        4 + caption_buttons,
        "only these lit: {:?}",
        lit_rects.iter().map(|r| r.x_range()).collect::<Vec<_>>()
    );
}

/// A shortcut that cannot be used is dimmed, not outlined.
///
/// egui draws a disabled widget from `noninteractive`, which is a
/// different entry from the four a hover walks through.
#[test]
fn a_disabled_shortcut_has_no_border() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&MenuModel {
        roots: vec![MenuNode::submenu(
            "Emulation",
            vec![MenuNode::item_enabled(
                MenuId::Restart,
                "Restart Emulation",
                false,
            )],
        )
        .with_status("No firmware", MenuIcon::Emulation)],
        ..Default::default()
    });
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    // The pill is the only thing in the strip that carries an outline.
    let outlined: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r)
                if r.stroke.width > 0.0 && (r.rect.height() - PILL_H).abs() < 1.0 =>
            {
                Some(r.rect.width())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        outlined.len(),
        1,
        "only the pill should be outlined, got {outlined:?}"
    );
}

/// A narrow window sheds shortcuts instead of drawing them over the pills.
#[test]
fn the_shortcuts_give_way_to_the_pills() {
    fn strip_at(width: f32) -> Vec<Rect> {
        let ctx = test_ctx();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![
                MenuNode::submenu(
                    "Emulation",
                    vec![MenuNode::item(MenuId::Restart, "Restart Emulation")],
                )
                .with_status("CDJ-3000", MenuIcon::Emulation)
                .with_detail("3.22"),
                MenuNode::submenu("Instances", vec![]).with_status("Slot 1", MenuIcon::Instances),
                MenuNode::submenu("Storage", vec![])
                    .with_icon(MenuIcon::Storage { mounted: false }),
                MenuNode::submenu("Network", vec![]).with_icon(MenuIcon::Network(NetKind::Nat)),
                MenuNode::submenu("Audio", vec![]).with_icon(MenuIcon::Audio { on: true }),
                MenuNode::submenu("View", vec![]).with_icon(MenuIcon::View),
            ],
            ..Default::default()
        });
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::new(0.0, 0.0),
                Vec2::new(width, 400.0),
            )),
            ..Default::default()
        };
        let out = run(&ctx, input.clone(), |ui| p.draw(ui));
        let _ = out;
        let out = run(&ctx, input, |ui| p.draw(ui));
        // The buttons' own plates: the artwork inside an icon is smaller
        // than any of them, and the bar's ground is taller.
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .filter(|r| {
                r.top() < BAR_HEIGHT
                    && (r.height() - PILL_H).abs() < 1.0
                    && r.width() >= ICON_BTN.x - 1.0
            })
            .collect()
    }

    fn overlaps(rects: &[Rect]) -> bool {
        rects.iter().enumerate().any(|(i, a)| {
            rects[i + 1..]
                .iter()
                .any(|b| a.intersects(*b) && a.width() > 1.0 && b.width() > 1.0)
        })
    }

    let wide = strip_at(900.0);
    assert!(!overlaps(&wide), "{wide:?}");
    let wide_n = wide.len();

    for width in [520.0, 460.0, 400.0, 340.0] {
        let got = strip_at(width);
        assert!(!overlaps(&got), "at {width}: {got:?}");
        assert!(
            got.len() <= wide_n,
            "at {width} the strip grew rather than shed: {got:?}"
        );
    }
    assert!(
        strip_at(340.0).len() < wide_n,
        "a narrow strip shed nothing"
    );
}

/// Every glyph the strip types has to exist in the font, or it draws as a
/// box. Marks missing from the font are painted instead; this test checks the
/// marks that are typed.
#[test]
fn the_typed_glyphs_all_exist() {
    let ctx = test_ctx();
    let _ = run(&ctx, Default::default(), |_| {});
    let font = FontId::proportional(FONT_ROW);
    for s in ["CDJ3K EMULATOR", "NAT", "Ctrl Q", "141 ms", "·", "—", "…"] {
        assert!(
            ctx.fonts_mut(|f| f.has_glyphs(&font, s)),
            "{s:?} has no glyph and would draw as a box"
        );
    }
}
