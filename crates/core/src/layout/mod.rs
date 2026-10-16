//! dono: C6
//! Modelo de layout em milímetros: conversão px↔mm, validação, bordas compartilhadas, perfis,
//! movimento em bloco e avisos de Dock/Hot Corner. Stubs do F2: assinaturas.

use crate::types::{
    DisplayInfo, DockInfo, EdgeLink, HotCorners, LayoutProfile, LayoutWarning, ProfileKey,
};

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("telas sobrepostas: {0} e {1}")]
    Overlap(String, String),
    #[error("tela duplicada: {0}")]
    DuplicateDisplay(String),
    #[error("não implementado")]
    NotImplemented,
}

/// Pixels (espaço da tela) → mm, por `size_mm` ou, sem ele, 96 * scale dpi.
pub fn px_to_mm(_display: &DisplayInfo, px: f32) -> f32 {
    px
}

pub fn mm_to_px(_display: &DisplayInfo, mm: f32) -> f32 {
    mm
}

pub fn validate(_profile: &LayoutProfile) -> Result<(), LayoutError> {
    Err(LayoutError::NotImplemented)
}

/// Bordas compartilhadas entre máquinas diferentes (tolerância de 0,5 mm).
pub fn shared_edges(_profile: &LayoutProfile) -> Vec<EdgeLink> {
    Vec::new()
}

/// Chave do perfil: ids ordenados das telas conectadas.
pub fn profile_key(displays: &[DisplayInfo]) -> ProfileKey {
    let mut ids: Vec<String> = displays.iter().map(|d| d.id.clone()).collect();
    ids.sort();
    ProfileKey(ids)
}

/// Perfil exato para `displays` ou o padrão (bloco Windows à direita do Mac, alinhado pelo topo).
pub fn select_profile(
    _profiles: &[LayoutProfile],
    _displays: &[DisplayInfo],
) -> Option<LayoutProfile> {
    None
}

/// Move o bloco Windows (ou só `display_id` se `windows_detached`) por `delta_mm`.
pub fn move_block(_profile: &mut LayoutProfile, _display_id: &str, _delta_mm: (f32, f32)) {}

pub fn dock_warnings(
    _profile: &LayoutProfile,
    _dock: DockInfo,
    _corners: HotCorners,
    _corner_dead_zone_mm: f32,
) -> Vec<LayoutWarning> {
    Vec::new()
}
