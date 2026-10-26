//! dono: WI1
//! Comandos de layout e telas (mesmo contrato do Mac). As escritas devolvem
//! `Err("layout_read_only")` quando este PC não pode editar (host fora do ar ou peer 0.2).
use relay_core::types::{DisplaysPayload, Host, LayoutBook, LayoutProfile, SaveLayoutResult};
use tauri::State;

use crate::state::AgentState;

#[tauri::command(rename_all = "snake_case")]
pub fn get_displays(state: State<'_, AgentState>) -> DisplaysPayload {
    state.backend.get_displays()
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_layout(state: State<'_, AgentState>) -> LayoutProfile {
    state.backend.get_layout()
}

#[tauri::command(rename_all = "snake_case")]
pub fn get_layout_book(state: State<'_, AgentState>) -> LayoutBook {
    state.backend.get_layout_book()
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_layout(
    state: State<'_, AgentState>,
    profile: LayoutProfile,
) -> Result<SaveLayoutResult, String> {
    state
        .backend
        .save_layout(profile)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_layout_profiles(
    state: State<'_, AgentState>,
    profiles: Vec<LayoutProfile>,
    active: String,
) -> Result<(), String> {
    state
        .backend
        .set_layout_profiles(profiles, active)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_layout_host(state: State<'_, AgentState>, host: Host) -> Result<(), String> {
    state
        .backend
        .set_layout_host(host)
        .map_err(|e| e.to_string())
}
