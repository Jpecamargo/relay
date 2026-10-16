//! dono: F2
//! Telas, perfis de layout e informações de Dock/Hot Corner.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Machine {
    Mac,
    Windows,
}

/// Uma tela física. Coordenadas e tamanho no espaço da própria máquina:
/// - `origin_px`/`size_px`: Mac em pontos (CGDisplayBounds), Windows em pixels físicos
///   (per-monitor v2). É o mesmo espaço de `warp`, `position` e `MouseAbs`.
/// - `scale`: Mac = backing scale (2.0 em Retina); Windows = DPI efetivo / 96. Usado no fallback
///   de px→mm (96 * scale dpi) quando `size_mm` falta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplayInfo {
    /// Id estável: UUID da tela no Mac; id do dispositivo de monitor no Windows.
    pub id: String,
    pub name: String,
    pub machine: Machine,
    pub origin_px: (i32, i32),
    pub size_px: (u32, u32),
    pub scale: f32,
    /// Tamanho físico em mm (CGDisplayScreenSize / EDID). `None` se desconhecido.
    pub size_mm: Option<(f32, f32)>,
    pub is_primary: bool,
    pub is_builtin: bool,
}

/// Retângulo no plano do layout, em milímetros.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RectMm {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlacedDisplay {
    pub display_id: String,
    pub machine: Machine,
    pub rect_mm: RectMm,
    /// Multiplicador do movimento do mouse sobre esta tela (1.0 = neutro).
    pub sensitivity: f32,
    /// Orientação manual da tela (0, 90, 180 ou 270 graus), gravada no perfil. `None` = automática:
    /// o layout segue o aspecto dos pixels que a máquina reporta. 90/270 deixam a tela em retrato
    /// (largura e altura em mm trocadas em relação à paisagem natural).
    #[serde(default)]
    pub rotation: Option<u16>,
}

/// Perfil escolhido pelo conjunto ordenado dos ids das telas conectadas (D28).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProfileKey(pub Vec<String>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayoutProfile {
    pub key: ProfileKey,
    pub name: String,
    pub displays: Vec<PlacedDisplay>,
    /// `true`: telas Windows movem separadamente (senão, em bloco).
    pub windows_detached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Trecho de borda compartilhado entre telas de máquinas diferentes.
/// `segment_mm`: intervalo (início, fim) ao longo da borda, em mm, no eixo da borda
/// (y para Left/Right, x para Top/Bottom) do plano do layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EdgeLink {
    pub from_display: String,
    pub to_display: String,
    pub side: Side,
    pub segment_mm: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DockOrientation {
    Bottom,
    Left,
    Right,
}

/// Estado do Dock do macOS (lido por `platform-macos::dock`, consumido por `layout::dock_warnings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DockInfo {
    pub autohide: bool,
    pub orientation: DockOrientation,
}

/// Hot Corners do macOS com ação configurada (`true` = ativo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HotCorners {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum LayoutWarningKind {
    DockAutohideEdge,
    HotCorner,
}

/// Aviso devolvido por `save_layout` (`layout::dock_warnings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayoutWarning {
    pub kind: LayoutWarningKind,
    pub message: String,
    pub display_id: Option<String>,
}
