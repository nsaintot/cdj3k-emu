//! The seam between the menu's content and the toolkit that draws it.
//!
//! A provider presents a [`MenuModel`] and reports what was clicked. It owns
//! no application state and decides nothing: two providers given the same
//! model must offer the same actions.

use super::id::MenuId;
use super::model::MenuModel;

pub trait MenuProvider {
    /// Present this model. It is called every frame with a new model, so a
    /// provider that holds native widgets must detect that nothing changed and
    /// skip the rebuild.
    fn apply(&mut self, model: &MenuModel);

    /// Actions raised since the last call, oldest first.
    fn poll(&mut self) -> Vec<MenuId>;

    /// The in-window provider, when this is one.
    ///
    /// Drawing needs an egui frame, which the trait does not carry. This method
    /// reaches the one provider that draws, without widening the interface.
    fn as_egui(&mut self) -> Option<&mut super::egui_provider::EguiProvider> {
        None
    }
}
