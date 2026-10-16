fn main() {
    // generate_context! exige que frontendDist exista; num checkout limpo ainda não houve `pnpm build`.
    std::fs::create_dir_all("../dist").expect("criar apps/*/dist");
    tauri_build::build();
}
