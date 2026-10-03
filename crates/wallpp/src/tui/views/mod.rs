pub mod general;
pub mod history;
pub mod source_edit;
pub mod sources;

pub use general::GeneralView;
pub use history::HistoryView;
pub use source_edit::SourceEditView;
pub use sources::{SourcesAction, SourcesView};
