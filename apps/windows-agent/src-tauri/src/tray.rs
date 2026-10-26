//! dono: WA1
//! Ícone da bandeja: clique esquerdo abre/fecha o painel (`panel`) junto à bandeja, clique direito
//! abre o menu (Abrir Relay, Atualização disponível quando houver, Encerrar).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime,
    image::Image,
    include_image,
    menu::{IsMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const ID_OPEN: &str = "open";
const ID_UPDATE: &str = "update";
const ID_QUIT: &str = "quit";

/// Evento que pede o modal da atualização à janela (mesmo nome de `@relay/ui`).
pub(crate) const OPEN_UPDATE_EVENT: &str = "relay:open-update";

/// Margem entre o painel e a borda da área de trabalho.
const PANEL_MARGIN: i32 = 8;

/// O clique na bandeja tira o foco do painel (que se esconde) antes de chegar aqui: sem esta janela
/// o mesmo clique que fecha o painel o reabriria.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

/// Quando o painel foi escondido por perder o foco.
static PANEL_BLURRED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Item "Atualização disponível" e o menu; só fica no menu enquanto `update_shown`.
struct TrayItems<R: Runtime> {
    menu: Menu<R>,
    update: MenuItem<R>,
    update_shown: Mutex<bool>,
}

/// Canto da área de trabalho (x, y, largura, altura) mais próximo do ponto da bandeja onde o painel
/// fica encostado, com `PANEL_MARGIN`. Funciona com a barra de tarefas em qualquer borda.
pub fn panel_origin(
    work: (i32, i32, u32, u32),
    panel: PhysicalSize<u32>,
    tray: PhysicalPosition<f64>,
) -> PhysicalPosition<i32> {
    let (wx, wy, ww, wh) = (work.0, work.1, work.2 as i32, work.3 as i32);
    let (pw, ph) = (panel.width as i32, panel.height as i32);
    let right = tray.x >= f64::from(wx) + f64::from(ww) / 2.0;
    let bottom = tray.y >= f64::from(wy) + f64::from(wh) / 2.0;
    let x = if right {
        wx + ww - pw - PANEL_MARGIN
    } else {
        wx + PANEL_MARGIN
    };
    let y = if bottom {
        wy + wh - ph - PANEL_MARGIN
    } else {
        wy + PANEL_MARGIN
    };
    PhysicalPosition::new(x.max(wx), y.max(wy))
}

/// Chamado quando o painel perde o foco.
pub fn panel_blurred<R: Runtime>(app: &AppHandle<R>) {
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = panel.hide();
    }
    if let Ok(mut at) = PANEL_BLURRED_AT.lock() {
        *at = Some(Instant::now());
    }
}

fn just_blurred() -> bool {
    PANEL_BLURRED_AT
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|t| t.elapsed() < REOPEN_GUARD)
}

fn toggle_panel<R: Runtime>(app: &AppHandle<R>, tray: PhysicalPosition<f64>) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    if just_blurred() {
        return;
    }
    let monitor = app
        .monitor_from_point(tray.x, tray.y)
        .ok()
        .flatten()
        .or_else(|| panel.current_monitor().ok().flatten());
    if let (Some(monitor), Ok(size)) = (monitor, panel.outer_size()) {
        let work = monitor.work_area();
        let pos = panel_origin(
            (
                work.position.x,
                work.position.y,
                work.size.width,
                work.size.height,
            ),
            size,
            tray,
        );
        let _ = panel.set_position(pos);
    }
    let _ = panel.show();
    let _ = panel.set_focus();
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(p) = app.get_webview_window("panel") {
        let _ = p.hide();
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Mostra (com a versão) ou tira do menu o item "Atualização disponível".
pub fn set_update_available<R: Runtime>(app: &AppHandle<R>, version: Option<&str>) {
    let Some(items) = app.try_state::<TrayItems<R>>() else {
        return;
    };
    let Ok(mut shown) = items.update_shown.lock() else {
        return;
    };
    match version {
        Some(v) => {
            let _ = items.update.set_text(update_label(v));
            if !*shown
                && items
                    .menu
                    .insert(&items.update as &dyn IsMenuItem<R>, 1)
                    .is_ok()
            {
                *shown = true;
            }
        }
        None => {
            if *shown
                && items
                    .menu
                    .remove(&items.update as &dyn IsMenuItem<R>)
                    .is_ok()
            {
                *shown = false;
            }
        }
    }
}

pub fn update_label(version: &str) -> String {
    format!("Atualização {version} disponível")
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, ID_OPEN, "Abrir Relay", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Encerrar", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let update = MenuItem::with_id(app, ID_UPDATE, update_label(""), true, None::<&str>)?;
    app.manage(TrayItems {
        menu: menu.clone(),
        update,
        update_shown: Mutex::new(false),
    });

    let icon: Image<'static> = include_image!("icons/icon.ico");
    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .tooltip("Relay")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            ID_OPEN => show_main_window(app),
            ID_UPDATE => {
                show_main_window(app);
                let _ = app.emit_to("main", OPEN_UPDATE_EVENT, ());
            }
            ID_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), position);
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1040);
    const PANEL: PhysicalSize<u32> = PhysicalSize::new(360, 420);

    #[test]
    fn painel_encosta_no_canto_inferior_direito_com_a_bandeja_embaixo() {
        let p = panel_origin(WORK, PANEL, PhysicalPosition::new(1800.0, 1060.0));
        assert_eq!((p.x, p.y), (1920 - 360 - 8, 1040 - 420 - 8));
    }

    #[test]
    fn painel_segue_a_bandeja_para_o_canto_superior_esquerdo() {
        let p = panel_origin(WORK, PANEL, PhysicalPosition::new(20.0, 10.0));
        assert_eq!((p.x, p.y), (8, 8));
    }

    #[test]
    fn area_de_trabalho_em_outro_monitor_e_respeitada() {
        let work = (-1920, 0, 1920, 1040);
        let p = panel_origin(work, PANEL, PhysicalPosition::new(-100.0, 1060.0));
        assert_eq!((p.x, p.y), (-360 - 8, 1040 - 420 - 8));
    }

    #[test]
    fn painel_maior_que_a_area_nao_sai_pela_origem() {
        let p = panel_origin((0, 0, 200, 200), PANEL, PhysicalPosition::new(190.0, 190.0));
        assert_eq!((p.x, p.y), (0, 0));
    }

    #[test]
    fn item_de_atualizacao_traz_a_versao() {
        assert_eq!(update_label("0.2.0"), "Atualização 0.2.0 disponível");
    }
}
