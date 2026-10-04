//! Sparkle 2, loaded at runtime from `Contents/Frameworks/Sparkle.framework`.
//!
//! Every slot's process runs an `SPUStandardUpdaterController`, so "Check for
//! Update…" works from any window; scheduled checks run in the lowest-numbered
//! open slot only. The delegate keeps Sparkle's two install hand-offs for the
//! UI thread, which calls back into it: every delegate method and every call
//! here runs on the main thread.

use std::cell::RefCell;

use block2::{Block, RcBlock};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_foundation::{NSBundle, NSString};

use super::Event;

/// `SPUUpdateCheckUpdatesInBackground`: a scheduled check.
const BACKGROUND_CHECK: isize = 1;

#[derive(Default)]
struct State {
    own: u32,
    controller: Option<Retained<AnyObject>>,
    _delegate: Option<Retained<Delegate>>,
    relaunch: Option<RcBlock<dyn Fn()>>,
    install: Option<RcBlock<dyn Fn()>>,
    /// The pending install is for a quit: Sparkle must not relaunch.
    quitting: bool,
    events: Vec<Event>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

declare_class!(
    struct Delegate;

    // SAFETY: NSObject has no subclassing requirements, the class has no
    // ivars and no Drop.
    unsafe impl ClassType for Delegate {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "CDJ3KEmuUpdaterDelegate";
    }

    impl DeclaredClass for Delegate {}

    // The SPUUpdaterDelegate methods the app answers; Sparkle asks
    // `respondsToSelector:` for each.
    unsafe impl Delegate {
        #[method_id(feedURLStringForUpdater:)]
        fn feed_url(&self, _updater: &AnyObject) -> Option<Retained<NSString>> {
            Some(NSString::from_str(&crate::appcast_url()))
        }

        #[method(updater:mayPerformUpdateCheck:error:)]
        fn may_check(
            &self,
            _updater: &AnyObject,
            check: isize,
            _error: *mut *mut AnyObject,
        ) -> Bool {
            let own = with(|s| s.own);
            Bool::new(check != BACKGROUND_CHECK || lowest_open_slot(own))
        }

        #[method(updater:shouldPostponeRelaunchForUpdate:untilInvokingBlock:)]
        fn postpone_relaunch(
            &self,
            _updater: &AnyObject,
            _item: &AnyObject,
            handler: &Block<dyn Fn()>,
        ) -> Bool {
            with(|s| {
                s.relaunch = Some(handler.copy());
                s.events.push(Event::RelaunchRequested);
            });
            Bool::YES
        }

        #[method(updaterShouldRelaunchApplication:)]
        fn should_relaunch(&self, _updater: &AnyObject) -> Bool {
            Bool::new(!with(|s| s.quitting))
        }

        #[method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:)]
        fn install_on_quit(
            &self,
            _updater: &AnyObject,
            _item: &AnyObject,
            handler: &Block<dyn Fn()>,
        ) -> Bool {
            with(|s| {
                s.install = Some(handler.copy());
                s.events.push(Event::InstallOnQuitReady);
            });
            Bool::YES
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        // SAFETY: NSObject's init on a fresh allocation.
        unsafe { msg_send_id![Self::alloc(), init] }
    }
}

/// Scheduled checks run in one window only.
fn lowest_open_slot(own: u32) -> bool {
    (1..own).all(|n| cdj3k_emu_storage::slot_holder(n).is_none())
}

pub(super) fn start(own: u32) -> bool {
    if with(|s| s.controller.is_some()) {
        return true;
    }
    let Some(framework) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Frameworks/Sparkle.framework")))
        .filter(|p| p.exists())
    else {
        return false;
    };
    let path = NSString::from_str(&framework.to_string_lossy());
    // SAFETY: a bundle path and its load, on the main thread.
    let loaded = unsafe { NSBundle::bundleWithPath(&path).is_some_and(|b| b.load()) };
    let Some(class) = loaded
        .then(|| AnyClass::get("SPUStandardUpdaterController"))
        .flatten()
    else {
        eprintln!("cdj3k-emu-update: Sparkle.framework would not load");
        return false;
    };
    // Before the controller starts: the delegate's first answer needs it.
    with(|s| s.own = own);
    let delegate = Delegate::new();
    // SAFETY: SPUStandardUpdaterController's designated initialiser; the
    // delegate is kept alive beside the controller, which holds it weakly.
    let controller: Option<Retained<AnyObject>> = unsafe {
        let obj: Allocated<AnyObject> = msg_send_id![class, alloc];
        msg_send_id![
            obj,
            initWithStartingUpdater: Bool::YES,
            updaterDelegate: &*delegate,
            userDriverDelegate: Option::<&AnyObject>::None
        ]
    };
    let Some(controller) = controller else {
        return false;
    };
    with(|s| {
        s.controller = Some(controller);
        s._delegate = Some(delegate);
    });
    true
}

pub(super) fn take_events() -> Vec<Event> {
    with(|s| std::mem::take(&mut s.events))
}

pub(super) fn check_now() {
    // Out of the borrow: Sparkle may call the delegate before returning.
    if let Some(controller) = with(|s| s.controller.clone()) {
        // SAFETY: an IBAction that takes a nil sender.
        unsafe {
            let _: () = msg_send![&*controller, checkForUpdates: Option::<&AnyObject>::None];
        }
    }
}

pub(super) fn proceed_relaunch() -> bool {
    match with(|s| s.relaunch.take()) {
        Some(handler) => {
            handler.call(());
            true
        }
        None => false,
    }
}

pub(super) fn install_pending() -> bool {
    with(|s| s.install.is_some())
}

pub(super) fn install_now(relaunch: bool) -> bool {
    // Sparkle's block runs once.
    let handler = with(|s| {
        s.quitting = !relaunch;
        s.install.take()
    });
    match handler {
        Some(handler) => {
            handler.call(());
            true
        }
        None => false,
    }
}
