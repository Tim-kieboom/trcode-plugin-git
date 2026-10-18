//! The Git pane of TerminalCode, as a plugin: the changed files in groups
//! (conflicts, staged changes, changes, untracked files) with the branch on top,
//! to move over, open in the editor, stage and unstage.
//!
//! The logic is in [`Viewer`], which answers messages with messages and knows
//! nothing of WebAssembly, so it is tested on the machine it is written on; the
//! last line makes it the module.

pub use viewer::Viewer;

#[cfg(test)]
mod tests;
mod viewer;

trcode_plugin_api::export_plugin!(Viewer);
