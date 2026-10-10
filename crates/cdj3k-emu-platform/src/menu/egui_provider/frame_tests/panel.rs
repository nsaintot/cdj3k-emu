//! The panels a menu opens: what they list, how wide they are, and what a
//! click on a row raises.

use super::*;

/// The hamburger opens, the rows are drawn, and clicking one raises its
/// action — the whole path a menu click takes, without a window.
#[test]
fn clicking_a_row_in_the_opened_menu_raises_its_action() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&model());

    // Frame one finds the hamburger; nothing is open yet.
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    assert!(
        !texts(&out).iter().any(|(t, _)| t == "Manage Emulation"),
        "the menu drew its rows before anything opened it"
    );
    let hamburger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);

    // Frame two opens it; the hamburger holds the menus, so what appears
    // is one opener per menu.
    let _ = run(&ctx, click_at(hamburger), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let opener = texts(&out)
        .into_iter()
        .find(|(t, _)| t == "Storage")
        .map(|(_, r)| r)
        .expect("the hamburger did not list the menus");

    // Frame three opens that menu, frame four sees its rows.
    let _ = run(&ctx, click_at(opener.center()), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let drawn = texts(&out);
    for want in [
        "Manage Emulation",
        "Service Mode",
        "SLOTS",
        "141 ms",
        "Quit",
    ] {
        assert!(
            drawn.iter().any(|(t, _)| t == want),
            "{want:?} missing from {:?}",
            drawn.iter().map(|(t, _)| t).collect::<Vec<_>>()
        );
    }

    let row = drawn
        .iter()
        .find(|(t, _)| t == "Service Mode")
        .map(|(_, r)| *r)
        .expect("the row was just asserted present");
    let _ = run(&ctx, click_at(row.center()), |ui| p.draw(ui));
    assert_eq!(p.poll(), vec![MenuId::ServiceMode]);
}

/// The update row heads the sandwich and leaves the pill it came from.
#[test]
fn the_update_row_is_first_in_the_sandwich_only() {
    let emulation = vec![
        MenuNode::item(MenuId::CheckForUpdate, "Check for Update…"),
        MenuNode::Separator,
        MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
    ];
    assert!(matches!(
        EguiProvider::menu_rows(&emulation).as_slice(),
        [MenuNode::Item {
            id: MenuId::ManageEmulation,
            ..
        }]
    ));

    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&MenuModel {
        roots: vec![
            MenuNode::submenu("Emulation", emulation),
            MenuNode::submenu("Storage", vec![]),
        ],
        ..Default::default()
    });
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
    let hamburger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
    let _ = run(&ctx, click_at(hamburger), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let drawn = texts(&out);
    let top = |want: &str| {
        drawn
            .iter()
            .find(|(t, _)| t == want)
            .map(|(_, r)| r.top())
            .unwrap_or_else(|| panic!("{want:?} missing from the sandwich"))
    };
    assert!(top("Check for Update…") < top("Storage"));
}

/// A panel is as wide as its widest row, not as wide as the window, with
/// a separator in it too.
#[test]
fn a_panel_fits_its_rows_rather_than_the_window() {
    let ctx = test_ctx();
    let mut measured = (0.0, 0.0);
    let _ = run(&ctx, Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            measured.0 = panel_width(ui, model().roots[0].children());
            measured.1 = panel_width(
                ui,
                &[MenuNode::check(
                    MenuId::PhysicalDisk(0),
                    "Some Very Long External Drive Name · 2 TB",
                    false,
                )
                .with_detail("disk4")],
            );
        });
    });
    assert_eq!(measured.0, PANEL_MIN_W, "short verbs stay at the minimum");
    assert!(
        measured.1 > PANEL_MIN_W && measured.1 <= PANEL_MAX_W,
        "a long device name widens the panel, to a point: {measured:?}"
    );
}

/// The pills have their own place in the strip, so the sandwich holds the
/// rest — not a second copy of them.
#[test]
fn the_sandwich_leaves_out_what_the_pills_already_reach() {
    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&MenuModel {
        roots: vec![
            model().roots[0].clone(),
            MenuNode::submenu(
                "Instances",
                vec![MenuNode::check(MenuId::Instance(1), "Slot 1", true)],
            )
            .with_status("Slot 1", MenuIcon::Instances),
            MenuNode::submenu(
                "View",
                vec![MenuNode::check(
                    MenuId::ScreenExtended,
                    "Extend Screen",
                    false,
                )],
            )
            .with_icon(MenuIcon::View),
        ],
        ..Default::default()
    });
    let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
    let _ = run(&ctx, click_at(burger), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(drawn.iter().any(|t| t == "View"), "{drawn:?}");
    assert!(drawn.iter().any(|t| t == "Quit"), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t == "Emulation"), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t == "Instances"), "{drawn:?}");
}

/// The sandwich ends in Quit; a pill's own menu does not repeat it.
///
/// The model keeps it in the first menu because the native bar needs it
/// there, so leaving it out is the provider's job.
#[test]
fn only_the_sandwich_carries_quit() {
    let with_quit = MenuModel {
        roots: vec![MenuNode::submenu(
            "Emulation",
            vec![
                MenuNode::item(MenuId::ManageEmulation, "Manage Emulation"),
                MenuNode::Separator,
                MenuNode::Predefined(Predefined::Quit),
            ],
        )
        .with_status("CDJ-3000", MenuIcon::Emulation)],
        ..Default::default()
    };
    let rows = EguiProvider::menu_rows(with_quit.roots[0].children());
    assert!(!rows
        .iter()
        .any(|n| matches!(n, MenuNode::Predefined(Predefined::Quit))));
    // And the separator that led to it goes with it.
    assert!(
        !matches!(rows.last(), Some(MenuNode::Separator)),
        "{rows:?}"
    );

    let ctx = test_ctx();
    let mut p = EguiProvider::default();
    p.apply(&with_quit);
    let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
    let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
    let _ = run(&ctx, click_at(burger), |ui| p.draw(ui));
    let out = run(&ctx, Default::default(), |ui| p.draw(ui));
    let drawn: Vec<_> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(drawn.iter().any(|t| t == "Quit"), "{drawn:?}");
}

/// Each View menu row is reachable and raises its own action.
#[test]
fn every_view_row_raises_its_own_action() {
    let rows = [
        (MenuId::ScreenExtended, "Extend Screen"),
        (MenuId::JogScreen, "External Jog Screen"),
        (MenuId::MainScreen, "External Main Screen"),
        (MenuId::DebugScreen, "Debug Panel"),
    ];
    for (id, label) in &rows {
        let ctx = test_ctx();
        let mut p = EguiProvider::default();
        p.apply(&MenuModel {
            roots: vec![MenuNode::submenu(
                "View",
                rows.iter()
                    .map(|(i, l)| MenuNode::check(i.clone(), *l, false))
                    .collect(),
            )
            .with_icon(MenuIcon::View)],
            ..Default::default()
        });
        let burger = Pos2::new(BAR_PAD_X + 15.0, BAR_HEIGHT / 2.0);
        let _ = run(&ctx, Default::default(), |ui| p.draw(ui));
        let _ = run(&ctx, click_at(burger), |ui| p.draw(ui));
        let out = run(&ctx, Default::default(), |ui| p.draw(ui));
        let opener = texts(&out)
            .into_iter()
            .find(|(t, _)| t == "View")
            .map(|(_, r)| r)
            .expect("the sandwich did not list View");
        let _ = run(&ctx, click_at(opener.center()), |ui| p.draw(ui));
        let out = run(&ctx, Default::default(), |ui| p.draw(ui));
        let row = texts(&out)
            .into_iter()
            .find(|(t, _)| t == label)
            .map(|(_, r)| r)
            .unwrap_or_else(|| panic!("{label:?} is not in the View menu"));
        let _ = run(&ctx, click_at(row.center()), |ui| p.draw(ui));
        assert_eq!(p.poll(), vec![id.clone()], "clicking {label:?}");
    }
}
