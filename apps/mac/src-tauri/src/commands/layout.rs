//! dono: MI1 (casco: MAC1)
use relay_core::types::{DisplaysPayload, LayoutProfile, SaveLayoutResult};
use tauri::State;

use crate::state::AppState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_displays(state: State<'_, AppState>) -> DisplaysPayload {
    state.backend.get_displays()
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_layout(state: State<'_, AppState>) -> LayoutProfile {
    state.backend.get_layout()
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_layout(
    state: State<'_, AppState>,
    profile: LayoutProfile,
) -> Result<SaveLayoutResult, String> {
    state.backend.save_layout(profile)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_layout_profiles(
    state: State<'_, AppState>,
    profiles: Vec<LayoutProfile>,
    active: String,
) -> Result<(), String> {
    state.backend.set_layout_profiles(profiles, active)
}
