//! The seam between the menu's content and the toolkit that draws it.
//!
//! A provider presents a [`MenuModel`] and reports what was clicked. It owns
//! no application state and decides nothing: two providers given the same
//! model must offer the same actions.

use super::id::MenuId;
use super::model::MenuModel;

pub trait MenuProvider {
    /// Present this model. Called every frame with a freshly built model, so a
    /// provider holding native widgets is responsible for recognising that
    /// nothing changed rather than rebuilding.
    fn apply(&mut self, model: &MenuModel);

    /// Actions raised since the last call, oldest first.
    fn poll(&mut self) -> Vec<MenuId>;

    /// The in-window provider, when this is one.
    ///
    /// Drawing needs an egui frame, which the trait does not carry; this
    /// reaches the one provider that draws without widening the interface.
    fn as_egui(&mut self) -> Option<&mut super::egui_provider::EguiProvider> {
        None
    }
}
