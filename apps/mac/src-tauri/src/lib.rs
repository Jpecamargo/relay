pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("erro ao executar o app Tauri");
}
